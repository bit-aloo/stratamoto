use std::{
    collections::HashMap,
    io::ErrorKind,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
pub use stratamoto_ir::compiler::ActionId;
use stratamoto_ir::{
    Protocol,
    compiler::{Action, CompiledProgram, ConnectionId, SessionId},
};

use crate::{
    error::Error,
    events::{ConnectionState, SetupState},
    transport::{Deployment, Transport},
};

/// How long the harness waits for an answer before calling a request unanswered.
pub const RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the harness waits for an answer that is not owed: a `SetupConnection` that was not
/// the first message on its connection. A role may stay silent, and waiting out the full
/// timeout each time would leave a real role's run dominated by waiting.
pub const UNOWED_RESPONSE_WAIT: Duration = Duration::from_millis(100);

/// What a role answered a `SetupConnection` with.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
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
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Session {
    pub connection: ConnectionId,
    pub protocol: Protocol,
    /// Whether the `SetupConnection` was the first message on its connection.
    pub first_on_connection: bool,
    pub response: SetupResponse,
}

/// Why an action was not attempted.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Prerequisite {
    /// The connection the action needed was never opened.
    ConnectionOpen(ConnectionId),
    /// The action was inside a block that runs only once the server agreed to the session's
    /// setup, and the server did not.
    SetupSuccess(SessionId),
}

/// What an action that awaited an answer was answered with.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// The action awaited nothing: a connect, a send, a wait.
    None,
    /// Never [`SetupResponse::Silence`]: silence is [`ActionOutcome::TimedOut`].
    Setup(SetupResponse),
    /// What a probe collected: the connection and message type of each frame.
    Probed(Vec<(ConnectionId, u8)>),
}

/// What became of one compiled action.
///
/// Exactly one is recorded per action, in the action's order, so that an oracle can ask
/// after every action it expects and find an answer, including that nothing happened and why.
/// Iterating only over what did happen would let missing work pass unnoticed.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum ActionOutcome {
    /// The action ran, and whatever it awaited arrived.
    Completed(Response),
    /// The action awaited an answer and none arrived before its deadline.
    TimedOut,
    /// The peer closed the connection before answering.
    TransportClosed,
    /// The transport failed in some other way, with the error's text.
    TransportError(String),
    /// A prerequisite of the action failed earlier, so it was not attempted.
    Skipped(Prerequisite),
    /// The harness could not carry the action out. A finding about the harness, never about
    /// the deployment.
    HarnessError(String),
}

/// The observable result of running a program.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Execution {
    /// One outcome per action of the program, in the program's order.
    pub outcomes: Vec<ActionOutcome>,
    pub sessions: HashMap<SessionId, Session>,
    /// Which role each connection was opened to.
    pub connection_roles: HashMap<ConnectionId, usize>,
    /// Frames that arrived outside a request, collected at each `Probe`.
    pub unsolicited: Vec<(ConnectionId, u8)>,
    /// Everything each connection received, in order, and the state it left it in.
    pub connections: HashMap<ConnectionId, ConnectionState>,
}

impl Execution {
    /// The outcome of an action, if the run recorded one.
    #[must_use]
    pub fn outcome(&self, action: ActionId) -> Option<&ActionOutcome> {
        self.outcomes.get(action)
    }

    /// Whether every action of the program has an outcome.
    #[must_use]
    pub fn is_total(&self, program: &CompiledProgram) -> bool {
        self.outcomes.len() == program.actions.len()
    }
}

/// Execute a compiled program against a deployment.
///
/// Actions run in order and each records exactly one outcome, so against a simulated
/// deployment the whole execution is a pure function of the program and the seed. Against a
/// real one it is only as reproducible as the role itself.
#[must_use]
pub fn run<D: Deployment>(deployment: &D, program: &CompiledProgram) -> Execution {
    let mut runner = Runner {
        deployment,
        connections: HashMap::new(),
        states: HashMap::new(),
        execution: Execution::default(),
    };

    let actions = &program.actions;
    let mut next = 0;
    while next < actions.len() {
        if let Some((unmet, end)) = runner.unmet_prerequisite(&actions[next]) {
            // The block is skipped whole: every action in it, the exit included, records
            // that it was, so that the trace says why nothing in there happened.
            let end = end.min(actions.len() - 1);
            for _ in next..=end {
                runner
                    .execution
                    .outcomes
                    .push(ActionOutcome::Skipped(unmet.clone()));
            }
            next = end + 1;
            continue;
        }
        let started = Instant::now();
        let outcome = runner.perform(&actions[next]);
        // The trace of a replay: what was done, for which instruction, what came of it and
        // how long it took. At debug, so that a campaign does not pay for it.
        tracing::debug!(
            "a{next} [i{}] {} -> {outcome:?} in {:.1?}",
            program
                .metadata
                .action_instructions
                .get(next)
                .copied()
                .unwrap_or_default(),
            describe(&actions[next]),
            started.elapsed()
        );
        runner.execution.outcomes.push(outcome);
        next += 1;
    }

    debug_assert!(runner.execution.is_total(program));
    runner.execution.connections = runner.states;
    runner.execution
}

/// An action as the trace shows it: a frame by its header and length rather than its bytes.
fn describe(action: &Action) -> String {
    match action {
        Action::Send {
            connection,
            extension_type,
            message_type,
            payload,
            ..
        } => format!(
            "Send {{ connection: {connection}, message_type: {message_type:#04x}, \
             extension_type: {extension_type:#06x}, payload: {} bytes }}",
            payload.len()
        ),
        other => format!("{other:?}"),
    }
}

struct Runner<'d, D: Deployment> {
    deployment: &'d D,
    connections: HashMap<ConnectionId, D::Transport<'d>>,
    states: HashMap<ConnectionId, ConnectionState>,
    execution: Execution,
}

impl<D: Deployment> Runner<'_, D> {
    /// For an action that opens a conditional block whose prerequisite the run has not met:
    /// what is unmet, and the last action to skip because of it.
    fn unmet_prerequisite(&self, action: &Action) -> Option<(Prerequisite, ActionId)> {
        match action {
            Action::EnterOnSetupSuccess { session, end } => (!self.established(*session))
                .then_some((Prerequisite::SetupSuccess(*session), *end)),
            _ => None,
        }
    }

    /// Whether the server agreed to the session's setup.
    fn established(&self, session: SessionId) -> bool {
        self.execution
            .sessions
            .get(&session)
            .is_some_and(|s| matches!(s.response, SetupResponse::Success { .. }))
    }

    /// Receive and dispatch frames on a connection until `done` holds of its state, or
    /// `timeout` passes with it not holding.
    fn pump(
        &mut self,
        connection: ConnectionId,
        timeout: Duration,
        done: impl Fn(&ConnectionState) -> bool,
    ) -> Result<(), ActionOutcome> {
        let (Some(link), Some(state)) = (
            self.connections.get_mut(&connection),
            self.states.get_mut(&connection),
        ) else {
            return Err(ActionOutcome::Skipped(Prerequisite::ConnectionOpen(
                connection,
            )));
        };
        let deadline = Instant::now() + timeout;
        loop {
            if done(state) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ActionOutcome::TimedOut);
            }
            match link.recv(remaining) {
                Ok(mut frame) => {
                    state.dispatch(&mut frame);
                }
                Err(Error::Timeout) => {
                    return if done(state) {
                        Ok(())
                    } else {
                        Err(ActionOutcome::TimedOut)
                    };
                }
                Err(e) => return Err(transport_failure(e)),
            }
        }
    }

    /// Carry out one action and say what became of it. Every path out of here is an outcome,
    /// which is what makes the execution total.
    fn perform(&mut self, action: &Action) -> ActionOutcome {
        match action {
            // Entering is decided by the loop, which has to skip a range; exiting is nothing.
            Action::EnterOnSetupSuccess { .. } | Action::ExitBlock => {
                ActionOutcome::Completed(Response::None)
            }

            Action::Connect { connection, role } => {
                let Some(link) = self.deployment.connect(*connection, *role) else {
                    tracing::debug!("cannot open connection {connection} to role {role}");
                    return ActionOutcome::TransportError(format!(
                        "could not open connection {connection} to role {role}"
                    ));
                };
                self.connections.insert(*connection, link);
                self.states.insert(*connection, ConnectionState::default());
                self.execution.connection_roles.insert(*connection, *role);
                ActionOutcome::Completed(Response::None)
            }

            Action::Send {
                connection,
                extension_type,
                message_type,
                channel_msg,
                payload,
            } => {
                let Some(link) = self.connections.get_mut(connection) else {
                    return ActionOutcome::Skipped(Prerequisite::ConnectionOpen(*connection));
                };
                match link.send(*extension_type, *message_type, *channel_msg, payload) {
                    Ok(()) => ActionOutcome::Completed(Response::None),
                    Err(e) => {
                        tracing::debug!("send failed on connection {connection}: {e}");
                        transport_failure(e)
                    }
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
                let Some(state) = self.states.get_mut(connection) else {
                    return ActionOutcome::Skipped(Prerequisite::ConnectionOpen(*connection));
                };
                state.setup = SetupState::Pending;
                let outcome = match self.pump(*connection, wait, |state| {
                    state.setup != SetupState::Pending
                }) {
                    Ok(()) => {
                        let response = match &self.states[connection].setup {
                            SetupState::Established {
                                used_version,
                                flags,
                            } => SetupResponse::Success {
                                used_version: *used_version,
                                flags: *flags,
                            },
                            SetupState::Rejected { flags, error_code } => SetupResponse::Error {
                                flags: *flags,
                                error_code: error_code.clone(),
                            },
                            SetupState::Unexpected { message_type } => SetupResponse::Unexpected {
                                message_type: *message_type,
                            },
                            SetupState::Unsent | SetupState::Pending => SetupResponse::Silence,
                        };
                        ActionOutcome::Completed(Response::Setup(response))
                    }
                    Err(failure) => failure,
                };
                let response = match &outcome {
                    ActionOutcome::Completed(Response::Setup(response)) => response.clone(),
                    _ => SetupResponse::Silence,
                };
                self.execution.sessions.insert(
                    *session,
                    Session {
                        connection: *connection,
                        protocol: *protocol,
                        first_on_connection: *first_on_connection,
                        response,
                    },
                );
                outcome
            }

            Action::AdvanceTime(duration) => {
                self.deployment.advance_time(*duration);
                ActionOutcome::Completed(Response::None)
            }

            Action::Probe => {
                let mut probed = Vec::new();
                for (id, link) in &mut self.connections {
                    let Some(state) = self.states.get_mut(id) else {
                        continue;
                    };
                    while let Ok(mut frame) = link.recv(Duration::ZERO) {
                        probed.push((*id, frame.header().msg_type()));
                        state.dispatch(&mut frame);
                    }
                }
                probed.sort_unstable();
                self.execution.unsolicited.extend(probed.iter().copied());
                ActionOutcome::Completed(Response::Probed(probed))
            }
        }
    }
}

/// The outcome a transport error amounts to.
fn transport_failure(error: Error) -> ActionOutcome {
    match error {
        Error::Timeout => ActionOutcome::TimedOut,
        Error::Io(e)
            if matches!(
                e.kind(),
                ErrorKind::UnexpectedEof
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
                    | ErrorKind::NotConnected
            ) =>
        {
            ActionOutcome::TransportClosed
        }
        other => ActionOutcome::TransportError(other.to_string()),
    }
}
