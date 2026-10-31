// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The one API, and the queue every caller stands in.
//!
//! [`Api`] is not thread-safe, and this is where that is honoured: it
//! lives on one thread, made there and dropped there, and a [`Hub::call`]
//! from any other thread is a message to it and a wait for the answer. A
//! transport thread blocks in `call` for as long as the method takes,
//! which is what it would do anyway; the long ones are jobs and return at
//! once.
//!
//! Events leave the other way. The API's sink is a fan-out over every
//! [`Subscriber`] a transport registered, called on the job's thread; a
//! subscriber that can no longer deliver says so by returning false and is
//! forgotten. A transport that knows its caller has gone - the stream it
//! read from ended - does not wait for that: [`Hub::subscribe`] hands back
//! a number, and [`Hub::unsubscribe_by_id`] takes the entry out at once.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use concat_api::{Api, ApiError, ErrorCode, Event, EventSink, Request, Response, Stopper};

/// A way to hand an event to one caller. Returns false once the caller is
/// gone, after which it is dropped.
pub type Subscriber = Arc<dyn Fn(&Event) -> bool + Send + Sync>;

/// One call waiting its turn.
struct Task {
    request: Request,
    respond: Box<dyn FnOnce(Response) + Send>,
}

/// The API's front door, shared by every transport. Cheap to clone.
#[derive(Clone)]
pub struct Hub {
    tasks: Arc<Mutex<Option<Sender<Task>>>>,
    /// Every registered subscriber with the number its `subscribe`
    /// returned, so one can be taken out without holding on to the
    /// closure itself: a gRPC stream guard knows only the number.
    subscribers: Arc<Mutex<Vec<(u64, Subscriber)>>>,
    /// Where subscriber numbers come from. Starts at one, so no number
    /// `subscribe` hands back can be mistaken for a zeroed default.
    next_subscriber: Arc<AtomicU64>,
}

impl Hub {
    /// Starts the API's thread. `make` runs on it, given the sink; if it
    /// fails, so does this.
    pub(crate) fn start(
        make: impl FnOnce(EventSink) -> Result<Api, String> + Send + 'static,
    ) -> Result<(Hub, JoinHandle<()>, Stopper), String> {
        let (tasks, queue) = mpsc::channel::<Task>();
        let subscribers: Arc<Mutex<Vec<(u64, Subscriber)>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: EventSink = {
            let subscribers = Arc::clone(&subscribers);
            Arc::new(move |event: Event| {
                // Called with the list unlocked: a caller slow to take an
                // event holds up the job's thread and nobody's subscribe.
                let listening: Vec<(u64, Subscriber)> = match subscribers.lock() {
                    Ok(list) => list.clone(),
                    Err(_) => return,
                };
                // Gone by number, not by pointer: the entry a
                // `unsubscribe_by_id` takes out in the meantime cannot
                // make the removal here land on the wrong one.
                let gone: Vec<u64> = listening
                    .iter()
                    .filter(|(_, subscriber)| !subscriber(&event))
                    .map(|(id, _)| *id)
                    .collect();
                if !gone.is_empty()
                    && let Ok(mut list) = subscribers.lock()
                {
                    list.retain(|(id, _)| !gone.contains(id));
                }
            })
        };
        let (made, ready) = mpsc::channel::<Result<Stopper, String>>();
        let thread = std::thread::Builder::new()
            .name("concat api".to_owned())
            .spawn(move || {
                let mut api = match make(sink) {
                    Ok(api) => {
                        let _ = made.send(Ok(api.stopper()));
                        api
                    }
                    Err(error) => {
                        let _ = made.send(Err(error));
                        return;
                    }
                };
                for task in queue {
                    // A panic in one method is that caller's error, not the
                    // end of the thread every later caller waits on. The
                    // documents a method had half-edited stay as they are,
                    // which is what a crash of the whole window would have
                    // left on disk anyway. Only where panics unwind: the
                    // CLI's server and the tests, not the window built
                    // with `--profile app`, which aborts on any panic.
                    let response = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        api.dispatch(task.request)
                    }))
                    .unwrap_or_else(|panic| {
                        let what = panic
                            .downcast_ref::<&str>()
                            .map(|text| (*text).to_owned())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "an unknown panic".to_owned());
                        Response::Error(ApiError::new(
                            ErrorCode::Failed,
                            format!("the method failed inside the server: {what}"),
                        ))
                    });
                    (task.respond)(response);
                }
                api.finish();
            })
            .map_err(|error| format!("could not start the API's thread: {error}"))?;
        let stopper = ready
            .recv()
            .unwrap_or_else(|_| Err("the API's thread ended before it began".to_owned()))?;
        Ok((
            Hub {
                tasks: Arc::new(Mutex::new(Some(tasks))),
                subscribers,
                next_subscriber: Arc::new(AtomicU64::new(1)),
            },
            thread,
            stopper,
        ))
    }

    /// Runs one request on the API's thread and waits for its response.
    pub fn call(&self, request: Request) -> Response {
        let gone = || {
            Response::Error(ApiError::new(
                ErrorCode::Failed,
                "the server is shutting down",
            ))
        };
        let (answer, wait) = mpsc::channel::<Response>();
        let task = Task {
            request,
            respond: Box::new(move |response| {
                let _ = answer.send(response);
            }),
        };
        let sent = match self.tasks.lock() {
            Ok(tasks) => tasks.as_ref().is_some_and(|tasks| tasks.send(task).is_ok()),
            Err(_) => false,
        };
        if !sent {
            return gone();
        }
        wait.recv().unwrap_or_else(|_| gone())
    }

    /// Registers a way to hand events to one caller, and returns its
    /// number, for [`Hub::unsubscribe_by_id`] when the caller goes. The
    /// number is the only handle some transports can keep: one whose
    /// stream outlives the call that subscribed it cannot hold on to the
    /// closure itself.
    pub fn subscribe(&self, subscriber: Subscriber) -> u64 {
        let id = self.next_subscriber.fetch_add(1, Ordering::SeqCst);
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push((id, subscriber));
        }
        id
    }

    /// Forgets a subscriber at once, for a caller that has hung up: the
    /// fan-out would only notice at the next event, which may be never.
    /// This one takes the number [`Hub::subscribe`] returned, where
    /// [`Hub::unsubscribe`] takes the closure itself.
    pub fn unsubscribe_by_id(&self, id: u64) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|(kept, _)| *kept != id);
        }
    }

    /// Forgets a subscriber at once, for a caller that has hung up: the
    /// fan-out would only notice at the next event, which may be never.
    pub fn unsubscribe(&self, subscriber: &Subscriber) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.retain(|(_, kept)| !Arc::ptr_eq(kept, subscriber));
        }
    }

    /// How many callers are receiving events.
    pub fn subscribers(&self) -> usize {
        self.subscribers.lock().map(|list| list.len()).unwrap_or(0)
    }

    /// Ends the queue. Calls after this are refused, and the API's thread
    /// finishes its jobs and returns.
    pub(crate) fn close(&self) {
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.take();
        }
    }
}
