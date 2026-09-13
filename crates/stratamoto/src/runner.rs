use std::{collections::HashMap, time::Duration};

use stratamoto_ir::{
    Protocol,
    compiler::{Action, CompiledProgram, ConnectionId, SessionId},
};
use stratum_core::parsers_sv2::{AnyMessage, CommonMessages};

use crate::{connection::Connection, deployment::Deployment, error::Error};

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
/// Actions run in order on the simulator's clock, so the whole execution is a pure function
/// of the program and the deployment's seed.
#[must_use]
pub fn run(deployment: &Deployment, program: &CompiledProgram) -> Execution {
    deployment.runtime().block_on(async {
        let mut connections: HashMap<ConnectionId, Connection> = HashMap::new();
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
                    let Some(link) = connections.get(connection) else {
                        log::debug!("send on unopened connection {connection}");
                        continue;
                    };
                    if let Err(e) = link
                        .send_frame(*extension_type, *message_type, *channel_msg, payload)
                        .await
                    {
                        log::debug!("send failed on connection {connection}: {e}");
                    }
                }

                Action::AwaitSetupResponse {
                    connection,
                    session,
                    protocol,
                } => {
                    let response = match connections.get_mut(connection) {
                        Some(link) => await_setup_response(link).await,
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

                Action::AdvanceTime(duration) => {
                    deterministic_simulator::time::sleep(*duration).await;
                }

                Action::Probe => {
                    for (id, link) in &mut connections {
                        while let Ok(mut frame) = link.recv_timeout(Duration::ZERO).await {
                            if let Ok(header) = frame.header() {
                                execution.unsolicited.push((*id, header.msg_type()));
                            }
                        }
                    }
                }
            }
        }

        execution
    })
}

async fn await_setup_response(link: &mut Connection) -> SetupResponse {
    let mut frame = match link.recv_timeout(RESPONSE_TIMEOUT).await {
        Ok(frame) => frame,
        Err(Error::Timeout) => return SetupResponse::Silence,
        Err(e) => {
            log::debug!("receive failed: {e}");
            return SetupResponse::Silence;
        }
    };

    let message_type = frame.header().map(|h| h.msg_type()).unwrap_or_default();
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
