// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
//! forgotten.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use concat_api::{Api, ApiError, ErrorCode, Event, EventSink, Request, Response};

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
    subscribers: Arc<Mutex<Vec<Subscriber>>>,
}

impl Hub {
    /// Starts the API's thread. `make` runs on it, given the sink; if it
    /// fails, so does this.
    pub(crate) fn start(
        make: impl FnOnce(EventSink) -> Result<Api, String> + Send + 'static,
    ) -> Result<(Hub, JoinHandle<()>), String> {
        let (tasks, queue) = mpsc::channel::<Task>();
        let subscribers: Arc<Mutex<Vec<Subscriber>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: EventSink = {
            let subscribers = Arc::clone(&subscribers);
            Arc::new(move |event: Event| {
                // Called with the list unlocked: a caller slow to take an
                // event holds up the job's thread and nobody's subscribe.
                let listening: Vec<Subscriber> = match subscribers.lock() {
                    Ok(list) => list.clone(),
                    Err(_) => return,
                };
                let gone: Vec<&Subscriber> = listening
                    .iter()
                    .filter(|subscriber| !subscriber(&event))
                    .collect();
                if !gone.is_empty()
                    && let Ok(mut list) = subscribers.lock()
                {
                    list.retain(|kept| !gone.iter().any(|left| Arc::ptr_eq(kept, left)));
                }
            })
        };
        let (made, ready) = mpsc::channel::<Result<(), String>>();
        let thread = std::thread::Builder::new()
            .name("concat api".to_owned())
            .spawn(move || {
                let mut api = match make(sink) {
                    Ok(api) => {
                        let _ = made.send(Ok(()));
                        api
                    }
                    Err(error) => {
                        let _ = made.send(Err(error));
                        return;
                    }
                };
                for task in queue {
                    let response = api.dispatch(task.request);
                    (task.respond)(response);
                }
                api.finish();
            })
            .map_err(|error| format!("could not start the API's thread: {error}"))?;
        ready
            .recv()
            .unwrap_or_else(|_| Err("the API's thread ended before it began".to_owned()))?;
        Ok((
            Hub {
                tasks: Arc::new(Mutex::new(Some(tasks))),
                subscribers,
            },
            thread,
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

    /// Registers a way to hand events to one caller.
    pub fn subscribe(&self, subscriber: Subscriber) {
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(subscriber);
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
