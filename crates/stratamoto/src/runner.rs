use std::{collections::HashMap, time::Duration};

use stratamoto_ir::{
    Protocol,
    compiler::{Action, ChannelSlot, CompiledProgram, ConnectionId, IdSource, SessionId},
};
use stratum_core::{
    binary_sv2::{GetSize, Serialize as Sv2Serialize, to_writer},
    mining_sv2::{
        MESSAGE_TYPE_OPEN_STANDARD_MINING_CHANNEL, MESSAGE_TYPE_SUBMIT_SHARES_STANDARD,
        SubmitSharesStandard,
    },
    parsers_sv2::{AnyMessage, CommonMessages, Mining},
};

use crate::{
    error::Error,
    transport::{Deployment, Transport},
};

/// How long the harness waits for an answer before calling a request unanswered.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the harness waits for an answer that is not owed: a `SetupConnection` that was not
/// the first message on its connection. A role may stay silent, and waiting out the full
/// timeout each time would leave a real role's run dominated by waiting.
pub const UNOWED_RESPONSE_WAIT: Duration = Duration::from_millis(100);

/// What a role answered a `SetupConnection` with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupResponse {
    Success {
        used_version: u16,
        flags: u32,
    },
    Error {
        flags: u32,
        error_code: String,
    },
    /// A frame arrived, but it was not a valid answer to `SetupConnection`.
    Unexpected {
        message_type: u8,
    },
    /// Nothing arrived before the timeout.
    Silence,
}

/// A session as the program set it up, paired with what the role answered.
#[derive(Debug, Clone)]
pub struct Session {
    pub connection: ConnectionId,
    pub protocol: Protocol,
    /// Whether the `SetupConnection` was the first message on its connection.
    pub first_on_connection: bool,
    pub response: SetupResponse,
}

/// What a role answered an `OpenStandardMiningChannel` with.
///
/// A successful open is answered by three messages: the success itself, the job the server
/// built from its latest template, and the prevhash that activates it. The job identifier is
/// taken from those, since nothing else can tell a program which job to mine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelOutcome {
    Success {
        /// The identifier the server echoed, which section 5.3.2 pairs with the request.
        request_id: u32,
        channel_id: u32,
        group_channel_id: u32,
        job_id: Option<u32>,
    },
    Error {
        request_id: u32,
        error_code: String,
    },
    Unexpected {
        message_type: u8,
    },
    Silence,
}

/// What a role answered a share with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShareOutcome {
    Success {
        channel_id: u32,
        last_sequence_number: u32,
    },
    Error {
        channel_id: u32,
        sequence_number: u32,
        error_code: String,
    },
    /// Nothing came back, which the protocol allows: shares are acknowledged in batches, so a
    /// valid share need not be answered on its own.
    Silence,
    Unexpected {
        message_type: u8,
    },
    /// The share named a channel the run never opened, so it was never sent.
    Unresolved,
}

/// The observable result of running a program.
#[derive(Debug, Clone, Default)]
pub struct Execution {
    pub sessions: HashMap<SessionId, Session>,
    /// Which role each connection was opened to.
    pub connection_roles: HashMap<ConnectionId, usize>,
    /// Frames that arrived outside a request, collected at each `Probe`.
    pub unsolicited: Vec<(ConnectionId, u8)>,
    /// What each channel a program opened was answered with.
    pub channels: HashMap<ChannelSlot, ChannelOutcome>,
    /// What each share a program submitted was answered with, in order.
    pub shares: Vec<ShareOutcome>,
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
                first_on_connection,
            } => {
                let wait = if *first_on_connection {
                    RESPONSE_TIMEOUT
                } else {
                    UNOWED_RESPONSE_WAIT
                };
                let Some(link) = connections.get_mut(connection) else {
                    // The connection was never opened, so there is nothing on it to judge.
                    // A liveness problem that kept it from opening is not this oracle's concern.
                    log::debug!("no session {session}: connection {connection} was never opened");
                    continue;
                };
                execution.sessions.insert(
                    *session,
                    Session {
                        connection: *connection,
                        protocol: *protocol,
                        first_on_connection: *first_on_connection,
                        response: await_setup_response(link, wait),
                    },
                );
            }

            Action::OpenStandardMiningChannel {
                connection,
                channel,
                request_id: _,
                payload,
                // Whether a frame of the program's own choosing came first is for the oracle
                // to weigh, not for the runner: the open is sent either way.
                after_raw_frame: _,
            } => {
                let Some(link) = connections.get_mut(connection) else {
                    log::debug!("open channel on unopened connection {connection}");
                    continue;
                };
                if let Err(e) =
                    link.send(0, MESSAGE_TYPE_OPEN_STANDARD_MINING_CHANNEL, false, payload)
                {
                    log::debug!("open channel failed on connection {connection}: {e}");
                }
                let outcome = await_channel_open(link);
                execution.channels.insert(*channel, outcome);
            }

            Action::SubmitSharesStandard {
                connection,
                channel_id,
                sequence_number,
                job_id,
                nonce,
                ntime,
                version,
            } => {
                let (Some(channel_id), Some(job_id)) = (
                    resolve(channel_id, &execution.channels),
                    resolve(job_id, &execution.channels),
                ) else {
                    // The channel it names was never opened, so there is no share to send.
                    execution.shares.push(ShareOutcome::Unresolved);
                    continue;
                };

                let Some(link) = connections.get_mut(connection) else {
                    execution.shares.push(ShareOutcome::Unresolved);
                    continue;
                };

                let share = SubmitSharesStandard {
                    channel_id,
                    sequence_number: *sequence_number,
                    job_id,
                    nonce: *nonce,
                    ntime: *ntime,
                    version: *version,
                };
                let Ok(payload) = encode(share) else {
                    execution.shares.push(ShareOutcome::Unresolved);
                    continue;
                };

                // A share is a channel message, so the frame carries the channel bit.
                if let Err(e) = link.send(0, MESSAGE_TYPE_SUBMIT_SHARES_STANDARD, true, &payload) {
                    log::debug!("share submission failed on connection {connection}: {e}");
                }
                execution.shares.push(await_share_response(link));
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

fn await_setup_response<T: Transport>(link: &mut T, wait: Duration) -> SetupResponse {
    let mut frame = match link.recv(wait) {
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

/// Resolve an identifier a message names, which is either written down or taken from what the
/// server assigned to a channel.
fn resolve(source: &IdSource, channels: &HashMap<ChannelSlot, ChannelOutcome>) -> Option<u32> {
    match source {
        IdSource::Literal(value) => Some(*value),
        IdSource::ChannelId(slot) => match channels.get(slot)? {
            ChannelOutcome::Success { channel_id, .. } => Some(*channel_id),
            _ => None,
        },
        IdSource::JobId(slot) => match channels.get(slot)? {
            ChannelOutcome::Success { job_id, .. } => *job_id,
            _ => None,
        },
    }
}

/// Read the answer to a channel open, and the job that follows a successful one.
fn await_channel_open<T: Transport>(link: &mut T) -> ChannelOutcome {
    let mut frame = match link.recv(RESPONSE_TIMEOUT) {
        Ok(frame) => frame,
        Err(_) => return ChannelOutcome::Silence,
    };

    let message_type = frame.header().msg_type();
    let mut outcome = match frame.message() {
        Ok(AnyMessage::Mining(Mining::OpenStandardMiningChannelSuccess(success))) => {
            ChannelOutcome::Success {
                request_id: success.request_id,
                channel_id: success.channel_id,
                group_channel_id: success.group_channel_id,
                job_id: None,
            }
        }
        Ok(AnyMessage::Mining(Mining::OpenMiningChannelError(error))) => ChannelOutcome::Error {
            request_id: error.request_id,
            error_code: String::from_utf8_lossy(error.error_code.as_ref()).into_owned(),
        },
        _ => ChannelOutcome::Unexpected { message_type },
    };

    if !matches!(outcome, ChannelOutcome::Success { .. }) {
        return outcome;
    }

    // The job and the prevhash follow the success on the same connection. Either names the job
    // the channel is to mine, so the first one that arrives settles it.
    for _ in 0..2 {
        let Ok(mut frame) = link.recv(RESPONSE_TIMEOUT) else {
            break;
        };
        let announced = match frame.message() {
            Ok(AnyMessage::Mining(Mining::NewMiningJob(job))) => Some(job.job_id),
            Ok(AnyMessage::Mining(Mining::SetNewPrevHash(prev_hash))) => Some(prev_hash.job_id),
            _ => None,
        };
        if let (Some(announced), ChannelOutcome::Success { job_id, .. }) = (announced, &mut outcome)
            && job_id.is_none()
        {
            *job_id = Some(announced);
        }
    }

    outcome
}

/// Read whatever a share drew, without insisting on an answer: a valid share is acknowledged
/// in batches, so silence is a legitimate outcome rather than a missing one.
fn await_share_response<T: Transport>(link: &mut T) -> ShareOutcome {
    let mut frame = match link.recv(UNOWED_RESPONSE_WAIT) {
        Ok(frame) => frame,
        Err(_) => return ShareOutcome::Silence,
    };

    let message_type = frame.header().msg_type();
    match frame.message() {
        Ok(AnyMessage::Mining(Mining::SubmitSharesSuccess(success))) => ShareOutcome::Success {
            channel_id: success.channel_id,
            last_sequence_number: success.last_sequence_number,
        },
        Ok(AnyMessage::Mining(Mining::SubmitSharesError(error))) => ShareOutcome::Error {
            channel_id: error.channel_id,
            sequence_number: error.sequence_number,
            error_code: String::from_utf8_lossy(error.error_code.as_ref()).into_owned(),
        },
        _ => ShareOutcome::Unexpected { message_type },
    }
}

fn encode<T: Sv2Serialize + GetSize>(message: T) -> Result<Vec<u8>, ()> {
    let mut payload = vec![0u8; message.get_size()];
    to_writer(message, &mut payload).map_err(|_| ())?;
    Ok(payload)
}
