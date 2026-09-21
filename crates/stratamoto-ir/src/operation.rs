use std::{fmt, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{Protocol, Variable, errors::ProgramValidationError};

/// `Operation` is a single action a program can perform on a deployment.
///
/// The variable types an operation consumes and produces are what encode the relationship
/// between messages: a session exists only inside the block that waits for the server to
/// agree to a `SetupConnection`, so nothing that needs one can run on a connection that was
/// never set up.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Operation {
    /// No operation, used for minimization.
    Nop {
        outputs: usize,
        inner_outputs: usize,
    },

    /// `Load*` operations lift data from the program's context into variables.
    LoadRole(usize),
    LoadConnection(usize),
    LoadVersion(u16),
    LoadFlags(u32),
    LoadPort(u16),
    LoadStr(String),
    LoadBytes(Vec<u8>),
    LoadDuration(Duration),

    /// Open a transport to a role.
    Connect,

    /// Begin building a `SetupConnection`. The block it opens holds the message under
    /// construction; every field keeps the value a client would send until an operation
    /// inside the block sets it.
    BeginBuildSetupConnection,
    SetVersions,
    SetFlags,
    SetEndpoint,
    SetDeviceInfo,
    /// Finalize the message for a subprotocol.
    EndBuildSetupConnection {
        protocol: Protocol,
    },

    /// Send a finalized `SetupConnection`. What it produces is an attempt: whether the server
    /// agreed is known only once it answers, and only then is there a session.
    SendSetupConnection {
        protocol: Protocol,
    },
    /// Open a block that runs only if the server answered the attempt with
    /// `SetupConnection.Success`. Inside it, and nowhere else, is the session it agreed to.
    BeginOnSetupSuccess {
        protocol: Protocol,
    },
    EndOnSetupSuccess,

    /// Send arbitrary bytes as a frame payload, bypassing message construction.
    SendRawFrame {
        message_type: u8,
        extension_type: u16,
    },

    AdvanceTime,

    /// Collect everything the deployment has sent since the last probe.
    Probe,
}

impl Operation {
    #[must_use]
    pub fn is_block_begin(&self) -> bool {
        matches!(
            self,
            Operation::BeginBuildSetupConnection | Operation::BeginOnSetupSuccess { .. }
        )
    }

    #[must_use]
    pub fn is_block_end(&self) -> bool {
        matches!(
            self,
            Operation::EndBuildSetupConnection { .. } | Operation::EndOnSetupSuccess
        )
    }

    #[must_use]
    pub fn is_matching_block_begin(&self, other: &Operation) -> bool {
        matches!(
            (self, other),
            (
                Operation::EndBuildSetupConnection { .. },
                Operation::BeginBuildSetupConnection
            ) | (
                Operation::EndOnSetupSuccess,
                Operation::BeginOnSetupSuccess { .. }
            )
        )
    }

    /// Variables produced in the scope the operation is written in.
    #[must_use]
    pub fn get_output_variables(&self) -> Vec<Variable> {
        match self {
            Operation::Nop { outputs, .. } => vec![Variable::Nop; *outputs],

            Operation::LoadRole(_) => vec![Variable::Role],
            Operation::LoadConnection(_) => vec![Variable::Connection],
            Operation::LoadVersion(_) => vec![Variable::Version],
            Operation::LoadFlags(_) => vec![Variable::Flags],
            Operation::LoadPort(_) => vec![Variable::Port],
            Operation::LoadStr(_) => vec![Variable::Str],
            Operation::LoadBytes(_) => vec![Variable::Bytes],
            Operation::LoadDuration(_) => vec![Variable::Duration],

            Operation::Connect => vec![Variable::Connection],

            Operation::EndBuildSetupConnection { protocol } => {
                vec![Variable::ConstSetupConnection(*protocol)]
            }
            Operation::SendSetupConnection { protocol } => {
                vec![Variable::SetupAttempt(*protocol)]
            }

            Operation::BeginBuildSetupConnection
            | Operation::BeginOnSetupSuccess { .. }
            | Operation::EndOnSetupSuccess
            | Operation::SetVersions
            | Operation::SetFlags
            | Operation::SetEndpoint
            | Operation::SetDeviceInfo
            | Operation::SendRawFrame { .. }
            | Operation::AdvanceTime
            | Operation::Probe => vec![],
        }
    }

    /// Variables produced inside the scope a block beginning opens.
    #[must_use]
    pub fn get_inner_output_variables(&self) -> Vec<Variable> {
        match self {
            Operation::Nop { inner_outputs, .. } => vec![Variable::Nop; *inner_outputs],
            Operation::BeginBuildSetupConnection => vec![Variable::MutSetupConnection],
            Operation::BeginOnSetupSuccess { protocol } => vec![Variable::Session(*protocol)],
            _ => vec![],
        }
    }

    #[must_use]
    pub fn get_input_variables(&self) -> Vec<Variable> {
        match self {
            Operation::Connect => vec![Variable::Role],

            Operation::SetVersions => vec![
                Variable::MutSetupConnection,
                Variable::Version,
                Variable::Version,
            ],
            Operation::SetFlags => vec![Variable::MutSetupConnection, Variable::Flags],
            Operation::SetEndpoint => {
                vec![Variable::MutSetupConnection, Variable::Str, Variable::Port]
            }
            Operation::SetDeviceInfo => vec![
                Variable::MutSetupConnection,
                Variable::Str,
                Variable::Str,
                Variable::Str,
                Variable::Str,
            ],
            Operation::EndBuildSetupConnection { .. } => vec![Variable::MutSetupConnection],

            Operation::SendSetupConnection { protocol } => vec![
                Variable::Connection,
                Variable::ConstSetupConnection(*protocol),
            ],
            Operation::BeginOnSetupSuccess { protocol } => vec![Variable::SetupAttempt(*protocol)],
            Operation::EndOnSetupSuccess => vec![],
            Operation::SendRawFrame { .. } => vec![Variable::Connection, Variable::Bytes],

            Operation::AdvanceTime => vec![Variable::Duration],

            Operation::Nop { .. }
            | Operation::LoadRole(_)
            | Operation::LoadConnection(_)
            | Operation::LoadVersion(_)
            | Operation::LoadFlags(_)
            | Operation::LoadPort(_)
            | Operation::LoadStr(_)
            | Operation::LoadBytes(_)
            | Operation::LoadDuration(_)
            | Operation::BeginBuildSetupConnection
            | Operation::Probe => vec![],
        }
    }

    #[must_use]
    pub fn num_outputs(&self) -> usize {
        self.get_output_variables().len()
    }

    #[must_use]
    pub fn num_inner_outputs(&self) -> usize {
        self.get_inner_output_variables().len()
    }

    #[must_use]
    pub fn num_inputs(&self) -> usize {
        self.get_input_variables().len()
    }

    pub fn check_input_types(&self, got: &[Variable]) -> Result<(), ProgramValidationError> {
        let expected = self.get_input_variables();
        if got.len() != expected.len() {
            return Err(ProgramValidationError::InvalidNumberOfInputs {
                is: got.len(),
                expected: expected.len(),
            });
        }
        for (got, expected) in got.iter().zip(expected.iter()) {
            if got != expected {
                return Err(ProgramValidationError::InvalidVariableType {
                    is: got.clone(),
                    expected: expected.clone(),
                });
            }
        }
        Ok(())
    }
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::Nop { .. } => write!(f, "Nop"),
            Operation::LoadRole(i) => write!(f, "LoadRole({i})"),
            Operation::LoadConnection(i) => write!(f, "LoadConnection({i})"),
            Operation::LoadVersion(v) => write!(f, "LoadVersion({v})"),
            Operation::LoadFlags(v) => write!(f, "LoadFlags(0x{v:08x})"),
            Operation::LoadPort(v) => write!(f, "LoadPort({v})"),
            Operation::LoadStr(v) => write!(f, "LoadStr({v:?})"),
            Operation::LoadBytes(v) => write!(f, "LoadBytes({} bytes)", v.len()),
            Operation::LoadDuration(v) => write!(f, "LoadDuration({v:?})"),
            Operation::Connect => write!(f, "Connect"),
            Operation::BeginBuildSetupConnection => write!(f, "BeginBuildSetupConnection"),
            Operation::SetVersions => write!(f, "SetVersions"),
            Operation::SetFlags => write!(f, "SetFlags"),
            Operation::SetEndpoint => write!(f, "SetEndpoint"),
            Operation::SetDeviceInfo => write!(f, "SetDeviceInfo"),
            Operation::EndBuildSetupConnection { protocol } => {
                write!(f, "EndBuildSetupConnection<{protocol}>")
            }
            Operation::SendSetupConnection { protocol } => {
                write!(f, "SendSetupConnection<{protocol}>")
            }
            Operation::BeginOnSetupSuccess { protocol } => {
                write!(f, "BeginOnSetupSuccess<{protocol}>")
            }
            Operation::EndOnSetupSuccess => write!(f, "EndOnSetupSuccess"),
            Operation::SendRawFrame {
                message_type,
                extension_type,
            } => write!(
                f,
                "SendRawFrame(0x{message_type:02x}, 0x{extension_type:04x})"
            ),
            Operation::AdvanceTime => write!(f, "AdvanceTime"),
            Operation::Probe => write!(f, "Probe"),
        }
    }
}
