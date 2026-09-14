use std::{collections::HashMap, time::Duration};

use stratamoto_ir::{
    Protocol,
    compiler::{Action, CompiledProgram, ConnectionId, SessionId},
};
use stratum_core::parsers_sv2::{AnyMessage, CommonMessages};

use crate::{
    error::Error,
    transport::{Deployment, Transport},
};

/// How long the harness waits for an answer before calling a request unanswered.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a role answered a `SetupConnection` with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupResponse {
    Success { used_version: u16, flags: u32 },
    Error { flags: u32, error_code: String },
    /// A frame arrived, but it was not a valid answer to `SetupConnection`.
    Unexpected { message_type: u8 },
    /// Nothing arrived before the timeout.
    Silence,
}

/// A session as the program set it up, paired with what the role answered.
#[derive(Debug, Clone)]
pub struct Session {
    pub connection: ConnectionId,
    pub protocol: Protocol,
    pub response: SetupResponse,
}

/// The observable result of running a program.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    pub sessions: HashMap<SessionId, Session>,
    /// Which role each connection was opened to.
    pub connection_roles: HashMap<ConnectionId, usize>,
    /// Frames that arrived outside a request, collected at each `Probe`.
    pub unsolicited: Vec<(ConnectionId, u8)>,
}

/// Execute a compiled program against a deployment.
///
/// Actions run in order, so against a simulated deployment the whole execution is a pure
/// function of the program and the seed. Against a real one it is only as reproducible as
/// the role itself.
#[must_use]
pub fn run<D: Deployment>(deployment: &D, program: &CompiledProgram) -> Execution {
    let mut connections: HashMap<ConnectionId, D::Transport<'_>> = HashMap::new();
    let mut execution = Execution::default();

    for action in &program.actions {
        match action {
            Action::Connect { connection, role } => {
                let Some(link) = deployment.connect(*connection, *role) else {
                    log::debug!("cannot address connection {connection} to role {role}");
                    continue;
                };
                connections.insert(*connection, link);
                execution.connection_roles.insert(*connection, *role);
            }

            Action::Send {
                connection,
                extension_type,
                message_type,
                channel_msg,
                payload,
            } => {
                let Some(link) = connections.get_mut(connection) else {
                    log::debug!("send on unopened connection {connection}");
                    continue;
                };
                if let Err(e) = link.send(*extension_type, *message_type, *channel_msg, payload) {
                    log::debug!("send failed on connection {connection}: {e}");
                }
            }

            Action::AwaitSetupResponse {
                connection,
                session,
                protocol,
            } => {
                let response = match connections.get_mut(connection) {
                    Some(link) => await_setup_response(link),
                    None => SetupResponse::Silence,
                };
                execution.sessions.insert(
                    *session,
                    Session {
                        connection: *connection,
                        protocol: *protocol,
                        response,
                    },
                );
            }

            Action::AdvanceTime(duration) => deployment.advance_time(*duration),

            Action::Probe => {
                for (id, link) in &mut connections {
                    while let Ok(frame) = link.recv(Duration::ZERO) {
                        execution.unsolicited.push((*id, frame.header().msg_type()));
                    }
                }
            }
        }
    }

    execution
}

fn await_setup_response<T: Transport>(link: &mut T) -> SetupResponse {
    let mut frame = match link.recv(RESPONSE_TIMEOUT) {
        Ok(frame) => frame,
        Err(Error::Timeout) => return SetupResponse::Silence,
        Err(e) => {
            log::debug!("receive failed: {e}");
            return SetupResponse::Silence;
        }
    };

    let message_type = frame.header().msg_type();
    match frame.message() {
        Ok(AnyMessage::Common(CommonMessages::SetupConnectionSuccess(success))) => {
            SetupResponse::Success {
                used_version: success.used_version,
                flags: success.flags,
            }
        }
        Ok(AnyMessage::Common(CommonMessages::SetupConnectionError(error))) => {
            SetupResponse::Error {
                flags: error.flags,
                error_code: String::from_utf8_lossy(error.error_code.as_ref()).into_owned(),
            }
        }
        _ => SetupResponse::Unexpected { message_type },
    }
}
