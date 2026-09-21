//! Everything a connection receives, decoded once, classified, and kept in order.
//!
//! An Sv2 connection carries the answers to requests and asynchronous notifications on one
//! ordered stream. Reading a fixed number of frames after a request misattributes one to the
//! other as soon as a server sends anything unprompted, so every frame goes through one
//! dispatcher that updates the connection's state, and a request is answered when the state
//! says so, whatever else arrived in between.

use serde::{Deserialize, Serialize};
use stratum_apps::stratum_core::parsers_sv2::{AnyMessage, CommonMessages, IsSv2Message};

use crate::connection::Frame;

/// One frame a connection received, as the dispatcher classified it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Event {
    SetupSuccess {
        used_version: u16,
        flags: u32,
    },
    SetupError {
        flags: u32,
        error_code: String,
    },
    /// The server asks the client to connect elsewhere.
    Reconnect {
        host: String,
        port: u16,
    },
    /// Decoded, of a type the harness does not model: anything of a subprotocol.
    Other {
        message_type: u8,
    },
    /// A type nothing defines, or an extension the harness does not speak.
    Unknown {
        extension_type: u16,
        message_type: u8,
        length: usize,
    },
    /// A known type whose payload did not decode.
    Malformed {
        message_type: u8,
        length: usize,
        reason: String,
    },
}

/// A frame whose header disagreed with its message.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HeaderIssue {
    pub message_type: u8,
    pub reason: String,
}

/// Where the connection's setup stands.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Default)]
pub enum SetupState {
    #[default]
    Unsent,
    /// A `SetupConnection` was sent and nothing has answered it.
    Pending,
    Established {
        used_version: u16,
        flags: u32,
    },
    Rejected {
        flags: u32,
        error_code: String,
    },
    /// The first frame after the setup was not an answer to it.
    Unexpected {
        message_type: u8,
    },
}

/// The state of one connection: what the program asked on it and what the server said.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ConnectionState {
    pub setup: SetupState,
    /// Every frame, in order of arrival.
    pub events: Vec<Event>,
    pub header_issues: Vec<HeaderIssue>,
}

impl ConnectionState {
    /// Take in one frame: validate its header, decode it once, update the state, and keep it.
    pub fn dispatch(&mut self, frame: &mut Frame) -> Event {
        let header = frame.header();
        let message_type = header.msg_type();
        let length = header.payload_length();
        let extension_type = header.ext_type_without_channel_msg();

        let event = if extension_type != 0 {
            Event::Unknown {
                extension_type,
                message_type,
                length,
            }
        } else {
            match frame.message() {
                Ok(message) => {
                    if message.channel_bit() != header.channel_msg() {
                        self.header_issues.push(HeaderIssue {
                            message_type,
                            reason: if header.channel_msg() {
                                "channel bit set on a message that is not a channel message"
                            } else {
                                "channel bit missing on a channel message"
                            }
                            .to_string(),
                        });
                    }
                    self.observe(&message, message_type)
                }
                Err(e) if known_message_type(message_type) => Event::Malformed {
                    message_type,
                    length,
                    reason: e.to_string(),
                },
                Err(_) => Event::Unknown {
                    extension_type,
                    message_type,
                    length,
                },
            }
        };

        self.events.push(event.clone());
        event
    }

    /// Classify a decoded message and update the state accordingly.
    ///
    /// Public so that the correlation can be tested with messages built by hand.
    pub fn observe(&mut self, message: &AnyMessage<'_>, message_type: u8) -> Event {
        let event = match message {
            AnyMessage::Common(CommonMessages::SetupConnectionSuccess(m)) => Event::SetupSuccess {
                used_version: m.used_version,
                flags: m.flags,
            },
            AnyMessage::Common(CommonMessages::SetupConnectionError(m)) => Event::SetupError {
                flags: m.flags,
                error_code: String::from_utf8_lossy(m.error_code.as_ref()).into_owned(),
            },
            AnyMessage::Common(CommonMessages::Reconnect(m)) => Event::Reconnect {
                host: String::from_utf8_lossy(m.new_host.as_ref()).into_owned(),
                port: m.new_port,
            },
            _ => Event::Other { message_type },
        };
        self.apply(&event, message_type);
        event
    }

    /// Update the state for an event. The first frame after a setup that is not its answer is
    /// what the setup got.
    fn apply(&mut self, event: &Event, message_type: u8) {
        if self.setup != SetupState::Pending {
            return;
        }
        self.setup = match event {
            Event::SetupSuccess {
                used_version,
                flags,
            } => SetupState::Established {
                used_version: *used_version,
                flags: *flags,
            },
            Event::SetupError { flags, error_code } => SetupState::Rejected {
                flags: *flags,
                error_code: error_code.clone(),
            },
            _ => SetupState::Unexpected { message_type },
        };
    }
}

/// Whether a message type is one the specification defines on extension zero.
#[must_use]
pub fn known_message_type(message_type: u8) -> bool {
    matches!(
        message_type,
        0x00..=0x04 | 0x10..=0x25 | 0x50..=0x60 | 0x70..=0x76
    )
}
