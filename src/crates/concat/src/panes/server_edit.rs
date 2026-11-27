// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The Scripts and apps page's edits to where the server listens and what
//! a caller presents, held apart from the settings in effect until Apply.
//!
//! A keystroke changes the [`Draft`] and nothing else: the server keeps
//! running on what it was started with, the preferences keep what was
//! last applied, and each field is checked as it is typed so the page can
//! say what is wrong beside it. Apply asks [`Draft::plan`] what the edits
//! amount to, and [`step`] what that takes of a server in the state it
//! is in: often nothing, a token put in place while everything runs on,
//! or a restart - the one thing that stops the jobs callers started.

use std::net::SocketAddr;

use super::settings::{join_listen, split_listen};

/// What is wrong with a value on the page, said beside its field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// The address is not one the server can listen on: "192.168." half
    /// typed, a name, a stray character.
    NotAnAddress,
    /// The port is not a number from 1 to 65535.
    NotAPort,
    /// The token is short or repetitive enough to guess.
    WeakToken,
}

/// The address field: empty is this computer, the way the placeholder
/// shows it; anything else must be an address the server can bind.
pub fn check_host(host: &str) -> Result<(), Note> {
    if host.trim().is_empty() {
        return Ok(());
    }
    join_listen(host, 1)
        .parse::<SocketAddr>()
        .map(drop)
        .map_err(|_| Note::NotAnAddress)
}

/// The port field. Zero would be whichever port is free, which no caller
/// could be told in advance, so it is not offered.
pub fn check_port(port: i32) -> Result<u16, Note> {
    u16::try_from(port)
        .ok()
        .filter(|port| *port > 0)
        .ok_or(Note::NotAPort)
}

/// The token field: empty means the server makes one up; anything else
/// must be strong enough to stand in front of it.
pub fn check_token(token: &str) -> Result<(), Note> {
    let token = token.trim();
    if token.is_empty() || concat_server::check_token(token).is_ok() {
        Ok(())
    } else {
        Err(Note::WeakToken)
    }
}

/// What the server was last told: the listen address saved, and the
/// token a caller presents now - the one saved, or the one the running
/// server made up when none was.
#[derive(Clone, Copy, Debug)]
pub struct Applied<'a> {
    pub listen: &'a str,
    pub token: &'a str,
}

/// The fields as edited and not yet applied. `None` is a field left
/// alone, which shows and keeps what is applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    pub host: Option<String>,
    pub port: Option<i32>,
    pub token: Option<String>,
}

/// What applying the draft changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The listen address to save: "127.0.0.1:7420".
    pub listen: String,
    /// The token to save, `None` when it stays as it is. Empty is no
    /// token chosen: the server makes one up.
    pub token: Option<String>,
    /// The server would listen somewhere else.
    pub moved: bool,
}

impl Draft {
    /// The address field as shown.
    pub fn host(&self, applied: Applied) -> String {
        self.host
            .clone()
            .unwrap_or_else(|| split_listen(applied.listen).0)
    }

    /// The port field as shown.
    pub fn port(&self, applied: Applied) -> i32 {
        self.port
            .unwrap_or_else(|| i32::from(split_listen(applied.listen).1))
    }

    /// The token field as shown.
    pub fn token(&self, applied: Applied) -> String {
        self.token
            .clone()
            .unwrap_or_else(|| applied.token.to_owned())
    }

    /// Each field's note, in the order the page shows them; `None` where
    /// the field is fine.
    pub fn notes(&self, applied: Applied) -> [Option<Note>; 3] {
        [
            check_host(&self.host(applied)).err(),
            check_port(self.port(applied)).err(),
            check_token(&self.token(applied)).err(),
        ]
    }

    /// What Apply would do with these edits: `Err` with the first note
    /// while a field is wrong, so a value that is never applied is never
    /// saved either, and `Ok(None)` when they come to what is applied.
    pub fn plan(&self, applied: Applied) -> Result<Option<Plan>, Note> {
        if let Some(note) = self.notes(applied).into_iter().flatten().next() {
            return Err(note);
        }
        let port = check_port(self.port(applied))?;
        let listen = join_listen(&self.host(applied), port);
        let moved = !same_listen(&listen, applied.listen);
        let token = self
            .token
            .as_deref()
            .map(str::trim)
            .filter(|token| *token != applied.token)
            .map(str::to_owned);
        if !moved && token.is_none() {
            return Ok(None);
        }
        Ok(Some(Plan {
            listen,
            token,
            moved,
        }))
    }

    /// Whether there is an edit Apply would act on, or one it would
    /// refuse: what shows the Apply row.
    pub fn pending(&self, applied: Applied) -> bool {
        !matches!(self.plan(applied), Ok(None))
    }
}

/// Two listen addresses the same server would bind alike: "[::1]:7420"
/// and "::1" with that port, or the same text with spaces round it.
fn same_listen(a: &str, b: &str) -> bool {
    match (
        a.trim().parse::<SocketAddr>(),
        b.trim().parse::<SocketAddr>(),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.trim() == b.trim(),
    }
}

/// The server as Apply finds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Server {
    /// The switch is off.
    Off,
    /// Listening.
    Running,
    /// The switch is on, and the server could not start.
    Failed,
}

/// What Apply does to the server for a plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Saved, for the next time the switch is turned on.
    Save,
    /// The token put in place on the running server; nothing stops.
    SwapToken,
    /// The server stopped and started on the new address: what cancels
    /// the jobs callers started, and is asked about first when any run.
    Restart,
    /// The switch is on and nothing listens: started on the new values.
    Start,
}

/// What a plan takes of the server: a restart only for an address the
/// running server does not listen on.
pub fn step(server: Server, plan: &Plan) -> Step {
    match server {
        Server::Off => Step::Save,
        Server::Failed => Step::Start,
        Server::Running if plan.moved => Step::Restart,
        Server::Running => Step::SwapToken,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTEN: &str = "127.0.0.1:7420";
    const TOKEN: &str = "a token long and varied enough 7f3Q";

    fn applied() -> Applied<'static> {
        Applied {
            listen: LISTEN,
            token: TOKEN,
        }
    }

    #[test]
    fn a_half_typed_address_is_not_one() {
        for host in [
            "192.168.",
            "192.168.1",
            "1.2.3.4.5",
            "local host",
            "10.0.0.x",
        ] {
            assert_eq!(check_host(host), Err(Note::NotAnAddress), "{host}");
        }
        for host in [
            "",
            " ",
            "127.0.0.1",
            "0.0.0.0",
            "192.168.1.9",
            "::1",
            "[::1]",
            "::",
        ] {
            assert_eq!(check_host(host), Ok(()), "{host}");
        }
    }

    #[test]
    fn a_port_is_one_to_65535() {
        assert_eq!(check_port(1), Ok(1));
        assert_eq!(check_port(65535), Ok(65535));
        for port in [0, -1, 65536] {
            assert_eq!(check_port(port), Err(Note::NotAPort), "{port}");
        }
    }

    #[test]
    fn a_token_is_empty_or_strong() {
        assert_eq!(check_token(""), Ok(()));
        assert_eq!(check_token(TOKEN), Ok(()));
        for token in [
            "short",
            "abcabcabcabcabcabcabcabc",
            "aaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(check_token(token), Err(Note::WeakToken), "{token}");
        }
    }

    #[test]
    fn untouched_fields_show_what_is_applied_and_have_nothing_to_apply() {
        let draft = Draft::default();
        assert_eq!(draft.host(applied()), "127.0.0.1");
        assert_eq!(draft.port(applied()), 7420);
        assert_eq!(draft.token(applied()), TOKEN);
        assert_eq!(draft.plan(applied()), Ok(None));
        assert!(!draft.pending(applied()));
    }

    #[test]
    fn a_wrong_field_is_pending_but_has_no_plan() {
        let draft = Draft {
            host: Some("192.168.".to_owned()),
            ..Draft::default()
        };
        assert_eq!(
            draft.notes(applied()),
            [Some(Note::NotAnAddress), None, None]
        );
        assert_eq!(draft.plan(applied()), Err(Note::NotAnAddress));
        assert!(
            draft.pending(applied()),
            "the row shows, its Apply cannot be pressed"
        );

        let weak = Draft {
            token: Some("open sesame".to_owned()),
            ..Draft::default()
        };
        assert_eq!(weak.plan(applied()), Err(Note::WeakToken));
    }

    #[test]
    fn an_edit_typed_back_to_what_is_applied_is_nothing_to_apply() {
        let draft = Draft {
            host: Some(" 127.0.0.1 ".to_owned()),
            port: Some(7420),
            token: Some(TOKEN.to_owned()),
        };
        assert_eq!(draft.plan(applied()), Ok(None));
        assert!(!draft.pending(applied()));
        let ipv6 = Applied {
            listen: "[::1]:7420",
            token: TOKEN,
        };
        let bare = Draft {
            host: Some("::1".to_owned()),
            ..Draft::default()
        };
        assert_eq!(
            bare.plan(ipv6),
            Ok(None),
            "the brackets are the same address"
        );
    }

    #[test]
    fn a_token_alone_is_put_in_place_and_stops_nothing() {
        let draft = Draft {
            token: Some("another token, longer still 9kR".to_owned()),
            ..Draft::default()
        };
        let plan = draft.plan(applied()).expect("valid").expect("a change");
        assert!(!plan.moved);
        assert_eq!(plan.listen, LISTEN);
        assert_eq!(
            plan.token.as_deref(),
            Some("another token, longer still 9kR")
        );
        assert_eq!(step(Server::Running, &plan), Step::SwapToken);
        assert_eq!(step(Server::Off, &plan), Step::Save);
        assert_eq!(step(Server::Failed, &plan), Step::Start);
    }

    #[test]
    fn a_cleared_token_asks_the_server_to_make_one_up() {
        let draft = Draft {
            token: Some(String::new()),
            ..Draft::default()
        };
        let plan = draft.plan(applied()).expect("valid").expect("a change");
        assert_eq!(plan.token.as_deref(), Some(""));
        assert_eq!(step(Server::Running, &plan), Step::SwapToken);
    }

    #[test]
    fn a_new_address_or_port_restarts_a_running_server_only() {
        for draft in [
            Draft {
                host: Some("0.0.0.0".to_owned()),
                ..Draft::default()
            },
            Draft {
                port: Some(7421),
                ..Draft::default()
            },
            Draft {
                port: Some(7421),
                token: Some("another token, longer still 9kR".to_owned()),
                ..Draft::default()
            },
        ] {
            let plan = draft.plan(applied()).expect("valid").expect("a change");
            assert!(plan.moved, "{draft:?}");
            assert_eq!(step(Server::Running, &plan), Step::Restart, "{draft:?}");
            assert_eq!(step(Server::Off, &plan), Step::Save, "{draft:?}");
            assert_eq!(step(Server::Failed, &plan), Step::Start, "{draft:?}");
        }
        let plan = Draft {
            port: Some(7421),
            ..Draft::default()
        }
        .plan(applied())
        .expect("valid")
        .expect("a change");
        assert_eq!(plan.listen, "127.0.0.1:7421");
        assert_eq!(plan.token, None, "the token stays as it is");
    }

    #[test]
    fn a_saved_value_that_is_wrong_says_so_without_an_edit() {
        let stale = Applied {
            listen: "192.168.:7420",
            token: TOKEN,
        };
        let draft = Draft::default();
        assert_eq!(draft.notes(stale)[0], Some(Note::NotAnAddress));
        assert!(draft.pending(stale));
    }
}
