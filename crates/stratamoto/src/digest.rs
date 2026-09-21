//! A normalized summary of what a run did, for telling a new behaviour from a repeat.
//!
//! The digest keeps what is observable and comparable across runs and drops what only says
//! which run it was: wall-clock timing and the text of errors an implementation is free to
//! choose. It also drops what only says how long the program was: counts are bucketed and
//! repeats collapsed, so that doing the same thing once more is not a new behaviour.
//!
//! A corpus does not key on the digest as a whole, which would keep every new combination of
//! old facts, but on the facts it is made of: see [`ExecutionDigest::features`].

use std::{
    collections::BTreeSet,
    hash::{DefaultHasher, Hash, Hasher},
};

use serde::{Deserialize, Serialize};
use stratamoto_ir::compiler::CompiledProgram;

use crate::{
    events::{ConnectionState, Event, SetupState},
    runner::{ActionOutcome, Execution, Prerequisite, SetupResponse},
};

/// The digest's format. Bumped when what goes into it changes, since a corpus keyed by an
/// older digest would then keep the wrong entries.
pub const VERSION: u32 = 2;

/// What the scenario made of a run, without the oracle's wording.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Verdict {
    Ok,
    Skip,
    /// Which oracle failed, by name.
    Fail(String),
    Infrastructure,
}

/// The class of answer a setup drew.
///
/// A success carries the flags the server settled on, which are few; an error carries only
/// whether it reported flags, since the bitmask it reports is the client's own bits echoed
/// back and a mutated client has as many of those as there are bits.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SetupClass {
    Success { used_version: u16, flags: u32 },
    Error { reported_flags: bool },
    Unexpected { message_type: u8 },
    Silence,
}

/// An event with what only identifies the run removed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EventClass {
    SetupSuccess,
    SetupError,
    Reconnect,
    Other {
        message_type: u8,
    },
    Unknown {
        extension_type: u16,
        message_type: u8,
    },
    Malformed {
        message_type: u8,
    },
}

impl EventClass {
    #[must_use]
    pub fn of(event: &Event) -> Self {
        match event {
            Event::SetupSuccess { .. } => EventClass::SetupSuccess,
            Event::SetupError { .. } => EventClass::SetupError,
            Event::Reconnect { .. } => EventClass::Reconnect,
            Event::Other { message_type } => EventClass::Other {
                message_type: *message_type,
            },
            Event::Unknown {
                extension_type,
                message_type,
                ..
            } => EventClass::Unknown {
                extension_type: *extension_type,
                message_type: *message_type,
            },
            Event::Malformed { message_type, .. } => EventClass::Malformed {
                message_type: *message_type,
            },
        }
    }
}

/// The class of an outcome, without what it was an outcome of.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OutcomeClass {
    Completed,
    TimedOut,
    TransportClosed,
    TransportError,
    SkippedConnection,
    SkippedSetup,
    HarnessError,
}

impl OutcomeClass {
    #[must_use]
    pub fn of(outcome: &ActionOutcome) -> Self {
        match outcome {
            ActionOutcome::Completed(_) => OutcomeClass::Completed,
            ActionOutcome::TimedOut => OutcomeClass::TimedOut,
            ActionOutcome::TransportClosed => OutcomeClass::TransportClosed,
            ActionOutcome::TransportError(_) => OutcomeClass::TransportError,
            ActionOutcome::Skipped(Prerequisite::ConnectionOpen(_)) => {
                OutcomeClass::SkippedConnection
            }
            ActionOutcome::Skipped(Prerequisite::SetupSuccess(_)) => OutcomeClass::SkippedSetup,
            ActionOutcome::HarnessError(_) => OutcomeClass::HarnessError,
        }
    }
}

/// Counts of what a run reached, bucketed: 0, 1, 2, then 3 to 4, 5 to 8, 9 to 16 and so on,
/// so that reaching more is new and reaching one more of the same is not.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Counts {
    pub connections: usize,
    pub established: usize,
}

/// What a run did, normalized.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExecutionDigest {
    pub version: u32,
    /// The distinct setups: the role asked, the protocol, whether the setup was the first
    /// message on its connection, and the class of answer. A set: a program that sets up the
    /// same thing twice has not reached anywhere new.
    pub setups: BTreeSet<(usize, stratamoto_ir::Protocol, bool, SetupClass)>,
    pub counts: Counts,
    /// The transitions the connections went through: each pair of consecutive event classes
    /// a connection saw, and the class it saw first, as a set. The edges of the state machine
    /// are what a run exercised; the path through them grows with the program and is not.
    pub transitions: BTreeSet<(Option<EventClass>, EventClass)>,
    /// The classes of frames that arrived outside any request, and of frames that could not
    /// be decoded, wherever they arrived.
    pub unsolicited: BTreeSet<EventClass>,
    pub malformed: BTreeSet<EventClass>,
    /// Whether any frame's header disagreed with its message.
    pub header_issues: bool,
    /// The ways actions ended.
    pub outcomes: BTreeSet<OutcomeClass>,
    /// Whether the deployment was still serving afterwards.
    pub alive: bool,
    pub verdict: Verdict,
}

impl ExecutionDigest {
    /// Digest a run of `program` that produced `execution`.
    #[must_use]
    pub fn of(
        _program: &CompiledProgram,
        execution: &Execution,
        alive: bool,
        verdict: Verdict,
    ) -> Self {
        let setups = execution
            .sessions
            .values()
            .map(|session| {
                let role = execution
                    .connection_roles
                    .get(&session.connection)
                    .copied()
                    .unwrap_or(usize::MAX);
                let class = match &session.response {
                    SetupResponse::Success {
                        used_version,
                        flags,
                    } => SetupClass::Success {
                        used_version: *used_version,
                        flags: *flags,
                    },
                    SetupResponse::Error { flags, .. } => SetupClass::Error {
                        reported_flags: *flags != 0,
                    },
                    SetupResponse::Unexpected { message_type } => SetupClass::Unexpected {
                        message_type: *message_type,
                    },
                    SetupResponse::Silence => SetupClass::Silence,
                };
                (role, session.protocol, session.first_on_connection, class)
            })
            .collect();

        let mut counts = Counts {
            connections: execution.connection_roles.len(),
            established: 0,
        };
        let mut transitions = BTreeSet::new();
        let mut malformed = BTreeSet::new();
        let mut header_issues = false;
        for state in execution.connections.values() {
            summarize(state, &mut counts, &mut transitions, &mut malformed);
            header_issues |= !state.header_issues.is_empty();
        }

        Self {
            version: VERSION,
            setups,
            counts: Counts {
                connections: bucket(counts.connections),
                established: bucket(counts.established),
            },
            transitions,
            unsolicited: unsolicited_classes(execution),
            malformed,
            header_issues,
            outcomes: execution.outcomes.iter().map(OutcomeClass::of).collect(),
            alive,
            verdict,
        }
    }

    /// The digest as one number, for telling two runs apart outright.
    #[must_use]
    pub fn hash(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        Hash::hash(self, &mut hasher);
        hasher.finish()
    }

    /// The digest as the facts it is made of, each as one number.
    ///
    /// A corpus keeps a program for any fact no earlier program showed, the way a coverage
    /// map keeps one for any edge: keyed by the whole digest it would keep every new
    /// combination of old facts, of which there are as many as there are subsets.
    #[must_use]
    pub fn features(&self) -> Vec<u64> {
        fn feature<T: Hash>(tag: &str, value: &T) -> u64 {
            let mut hasher = DefaultHasher::new();
            tag.hash(&mut hasher);
            value.hash(&mut hasher);
            hasher.finish()
        }
        let mut features = vec![feature("version", &self.version)];
        features.extend(self.setups.iter().map(|s| feature("setup", s)));
        features.push(feature("connections", &self.counts.connections));
        features.push(feature("established", &self.counts.established));
        features.extend(self.transitions.iter().map(|t| feature("transition", t)));
        features.extend(self.unsolicited.iter().map(|u| feature("unsolicited", u)));
        features.extend(self.malformed.iter().map(|m| feature("malformed", m)));
        features.push(feature("header_issues", &self.header_issues));
        features.extend(self.outcomes.iter().map(|o| feature("outcome", o)));
        features.push(feature("alive", &self.alive));
        features.push(feature("verdict", &self.verdict));
        features
    }
}

/// 0, 1 and 2 as they are; then the power of two the count is under.
fn bucket(count: usize) -> usize {
    if count < 3 {
        count
    } else {
        2 + (count - 1).ilog2() as usize
    }
}

/// The sequence with consecutive repeats collapsed.
fn collapsed<T: PartialEq>(items: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for item in items {
        if out.last() != Some(&item) {
            out.push(item);
        }
    }
    out
}

fn summarize(
    state: &ConnectionState,
    counts: &mut Counts,
    transitions: &mut BTreeSet<(Option<EventClass>, EventClass)>,
    malformed: &mut BTreeSet<EventClass>,
) {
    if matches!(state.setup, SetupState::Established { .. }) {
        counts.established += 1;
    }
    let mut previous = None;
    for class in collapsed(state.events.iter().map(EventClass::of)) {
        if matches!(
            class,
            EventClass::Unknown { .. } | EventClass::Malformed { .. }
        ) {
            malformed.insert(class.clone());
        }
        transitions.insert((previous.clone(), class.clone()));
        previous = Some(class);
    }
}

/// The classes of the frames a probe collected, matched back to the events by message type.
fn unsolicited_classes(execution: &Execution) -> BTreeSet<EventClass> {
    use stratum_apps::stratum_core::common_messages_sv2 as common;
    let mut classes = BTreeSet::new();
    for (connection, message_type) in &execution.unsolicited {
        let Some(state) = execution.connections.get(connection) else {
            continue;
        };
        let class = state
            .events
            .iter()
            .map(EventClass::of)
            .find(|class| {
                let of = match class {
                    EventClass::SetupSuccess => common::MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
                    EventClass::SetupError => common::MESSAGE_TYPE_SETUP_CONNECTION_ERROR,
                    EventClass::Reconnect => common::MESSAGE_TYPE_RECONNECT,
                    EventClass::Other { message_type }
                    | EventClass::Unknown { message_type, .. }
                    | EventClass::Malformed { message_type } => *message_type,
                };
                of == *message_type
            })
            .unwrap_or(EventClass::Other {
                message_type: *message_type,
            });
        classes.insert(class);
    }
    classes
}
