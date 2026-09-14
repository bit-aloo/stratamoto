use std::{fmt, time::Duration};

use serde::{Deserialize, Serialize};

use crate::{Protocol, Variable, errors::ProgramValidationError};

/// `Operation` is a single action a program can perform on a deployment.
///
/// The variable types an operation consumes and produces are what encode the relationship
/// between messages: a message can only be sent on a session that a `SetupConnection` for
/// its subprotocol produced.
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

    /// Open a new transport from the harness to a role.
    Connect,

    /// `SetupConnection` construction. The subprotocol is decided when the message is
    /// finalized, so the setters are shared by all subprotocols.
    BeginBuildSetupConnection,
    SetVersions,
    SetFlags,
    SetEndpoint,
    SetDeviceInfo,
    EndBuildSetupConnection {
        protocol: Protocol,
    },

    /// Send a `SetupConnection` and bind the upstream's answer to a session.
    SendSetupConnection {
        protocol: Protocol,
    },

    /// Mining protocol values.
    LoadRequestId(u32),
    /// A nominal hash rate, as the bits of the `f32` the wire carries. Bits rather than a
    /// float because an operation has to compare equal and hash, and because it lets a
    /// mutation reach the values a float has and an integer does not.
    LoadHashrate(u32),
    LoadTarget([u8; 32]),
    LoadChannelId(u32),
    LoadJobId(u32),
    LoadSequenceNumber(u32),
    LoadNonce(u32),
    LoadNtime(u32),
    LoadBlockVersion(u32),

    /// Open a standard mining channel on a mining session.
    ///
    /// The channel it produces is bound when the server answers, since the identifiers belong
    /// to the server, not to the program.
    OpenStandardMiningChannel,
    /// The identifier the server assigned to a channel.
    ChannelIdOf,
    /// The job the server announced on a channel.
    JobIdOf,
    /// Submit a share against a job on a channel.
    SubmitSharesStandard,

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
        matches!(self, Operation::BeginBuildSetupConnection)
    }

    #[must_use]
    pub fn is_block_end(&self) -> bool {
        matches!(self, Operation::EndBuildSetupConnection { .. })
    }

    #[must_use]
    pub fn is_matching_block_begin(&self, other: &Operation) -> bool {
        matches!(
            (self, other),
            (
                Operation::EndBuildSetupConnection { .. },
                Operation::BeginBuildSetupConnection
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
            Operation::SendSetupConnection { protocol } => vec![Variable::Session(*protocol)],

            Operation::LoadRequestId(_) => vec![Variable::RequestId],
            Operation::LoadHashrate(_) => vec![Variable::Hashrate],
            Operation::LoadTarget(_) => vec![Variable::Target],
            Operation::LoadChannelId(_) => vec![Variable::ChannelId],
            Operation::LoadJobId(_) => vec![Variable::JobId],
            Operation::LoadSequenceNumber(_) => vec![Variable::SequenceNumber],
            Operation::LoadNonce(_) => vec![Variable::Nonce],
            Operation::LoadNtime(_) => vec![Variable::Ntime],
            Operation::LoadBlockVersion(_) => vec![Variable::BlockVersion],

            Operation::OpenStandardMiningChannel => vec![Variable::Channel],
            Operation::ChannelIdOf => vec![Variable::ChannelId],
            Operation::JobIdOf => vec![Variable::JobId],

            Operation::BeginBuildSetupConnection
            | Operation::SetVersions
            | Operation::SetFlags
            | Operation::SetEndpoint
            | Operation::SetDeviceInfo
            | Operation::SendRawFrame { .. }
            | Operation::SubmitSharesStandard
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
            Operation::SendRawFrame { .. } => vec![Variable::Connection, Variable::Bytes],

            // A mining message may only be sent on a session that was set up for mining, and a
            // share may only name a channel and a job, which is what ties it to an open the
            // server answered.
            Operation::OpenStandardMiningChannel => vec![
                Variable::Session(Protocol::Mining),
                Variable::RequestId,
                Variable::Str,
                Variable::Hashrate,
                Variable::Target,
            ],
            Operation::ChannelIdOf | Operation::JobIdOf => vec![Variable::Channel],
            Operation::SubmitSharesStandard => vec![
                Variable::Session(Protocol::Mining),
                Variable::ChannelId,
                Variable::SequenceNumber,
                Variable::JobId,
                Variable::Nonce,
                Variable::Ntime,
                Variable::BlockVersion,
            ],

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
            | Operation::LoadRequestId(_)
            | Operation::LoadHashrate(_)
            | Operation::LoadTarget(_)
            | Operation::LoadChannelId(_)
            | Operation::LoadJobId(_)
            | Operation::LoadSequenceNumber(_)
            | Operation::LoadNonce(_)
            | Operation::LoadNtime(_)
            | Operation::LoadBlockVersion(_)
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
            Operation::SendRawFrame {
                message_type,
                extension_type,
            } => write!(f, "SendRawFrame(0x{message_type:02x}, 0x{extension_type:04x})"),
            Operation::LoadRequestId(v) => write!(f, "LoadRequestId({v})"),
            Operation::LoadHashrate(v) => {
                write!(f, "LoadHashrate({})", f32::from_bits(*v))
            }
            Operation::LoadTarget(_) => write!(f, "LoadTarget(..)"),
            Operation::LoadChannelId(v) => write!(f, "LoadChannelId({v})"),
            Operation::LoadJobId(v) => write!(f, "LoadJobId({v})"),
            Operation::LoadSequenceNumber(v) => write!(f, "LoadSequenceNumber({v})"),
            Operation::LoadNonce(v) => write!(f, "LoadNonce({v})"),
            Operation::LoadNtime(v) => write!(f, "LoadNtime({v})"),
            Operation::LoadBlockVersion(v) => write!(f, "LoadBlockVersion(0x{v:08x})"),
            Operation::OpenStandardMiningChannel => write!(f, "OpenStandardMiningChannel"),
            Operation::ChannelIdOf => write!(f, "ChannelIdOf"),
            Operation::JobIdOf => write!(f, "JobIdOf"),
            Operation::SubmitSharesStandard => write!(f, "SubmitSharesStandard"),
            Operation::AdvanceTime => write!(f, "AdvanceTime"),
            Operation::Probe => write!(f, "Probe"),
        }
    }
}
