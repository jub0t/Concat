// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Who has presented a wrong token lately, and how long each must wait.
//!
//! A minted token is 128 bits and no throttle is needed to keep it; a
//! chosen one passed [`crate::token::check`] and is still something a
//! person typed. So a caller gets a few honest mistakes, and after those
//! every wrong token doubles the wait before that address is heard again,
//! up to a minute. The right token clears the slate. The book is bounded:
//! past [`MAX_PEERS`] the quiet entries go first.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Wrong tokens forgiven before the waiting starts.
pub const FREE_FAILURES: u32 = 3;
/// The first wait, doubled at each failure after it.
pub const FIRST_WAIT: Duration = Duration::from_millis(250);
/// The longest wait.
pub const LONGEST_WAIT: Duration = Duration::from_secs(60);
/// How many addresses are remembered at once.
const MAX_PEERS: usize = 4096;
/// How long an address that stopped failing is remembered.
const MEMORY: Duration = Duration::from_secs(600);

/// Where a connection came from, as far as the throttle cares: an
/// address, or this machine through a Unix socket.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Peer {
    /// A TCP caller's address. The port is not part of it: a new
    /// connection is a new port.
    Ip(IpAddr),
    /// A Unix socket caller, whose access the socket file's permissions
    /// already decide.
    Local,
}

#[derive(Clone, Copy)]
struct Strikes {
    failures: u32,
    until: Instant,
    last: Instant,
}

/// The book of recent failures. Shared by every transport of a server.
#[derive(Default)]
pub struct Throttle {
    peers: Mutex<HashMap<Peer, Strikes>>,
}

impl Throttle {
    /// How long `peer` must still wait before a token from it is read;
    /// `None` when it need not.
    pub fn wait(&self, peer: Peer) -> Option<Duration> {
        let peers = self.peers.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let strikes = peers.get(&peer)?;
        strikes
            .until
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
    }

    /// `peer` presented a wrong token.
    pub fn failed(&self, peer: Peer) {
        let now = Instant::now();
        let mut peers = self.peers.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if peers.len() >= MAX_PEERS && !peers.contains_key(&peer) {
            peers.retain(|_, strikes| now.duration_since(strikes.last) < MEMORY);
            if peers.len() >= MAX_PEERS {
                // Every entry is fresh: an address flood. Forget the one
                // that has waited longest rather than grow.
                if let Some(oldest) = peers
                    .iter()
                    .min_by_key(|(_, strikes)| strikes.last)
                    .map(|(peer, _)| *peer)
                {
                    peers.remove(&oldest);
                }
            }
        }
        let strikes = peers.entry(peer).or_insert(Strikes {
            failures: 0,
            until: now,
            last: now,
        });
        strikes.failures = strikes.failures.saturating_add(1);
        strikes.last = now;
        strikes.until = now + wait_after(strikes.failures);
    }

    /// `peer` presented the right token: its slate is clean.
    pub fn succeeded(&self, peer: Peer) {
        self.peers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&peer);
    }
}

/// The wait after the `failures`th wrong token in a row.
pub fn wait_after(failures: u32) -> Duration {
    if failures <= FREE_FAILURES {
        return Duration::ZERO;
    }
    let doublings = (failures - FREE_FAILURES - 1).min(16);
    FIRST_WAIT
        .saturating_mul(1 << doublings)
        .min(LONGEST_WAIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_starts_after_the_free_failures_and_doubles_to_a_ceiling() {
        assert_eq!(wait_after(1), Duration::ZERO);
        assert_eq!(wait_after(FREE_FAILURES), Duration::ZERO);
        assert_eq!(wait_after(FREE_FAILURES + 1), FIRST_WAIT);
        assert_eq!(wait_after(FREE_FAILURES + 2), FIRST_WAIT * 2);
        assert_eq!(wait_after(FREE_FAILURES + 3), FIRST_WAIT * 4);
        assert_eq!(wait_after(1000), LONGEST_WAIT);
    }

    #[test]
    fn a_peer_waits_after_its_failures_and_the_right_token_clears_it() {
        let throttle = Throttle::default();
        let peer = Peer::Ip("192.0.2.1".parse().expect("an address"));
        let other = Peer::Ip("192.0.2.2".parse().expect("an address"));
        for _ in 0..FREE_FAILURES {
            throttle.failed(peer);
            assert_eq!(throttle.wait(peer), None, "forgiven");
        }
        throttle.failed(peer);
        assert!(throttle.wait(peer).is_some());
        assert_eq!(throttle.wait(other), None, "only the one that failed");
        throttle.succeeded(peer);
        assert_eq!(throttle.wait(peer), None);
    }
}
