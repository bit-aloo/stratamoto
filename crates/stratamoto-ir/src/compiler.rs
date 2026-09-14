use std::{
    collections::{HashMap, HashSet},
    fmt,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use stratum_core::{
    binary_sv2::{GetSize, Serialize as Sv2Serialize, Str0255, to_writer},
    common_messages_sv2::{MESSAGE_TYPE_SETUP_CONNECTION, SetupConnection},
    mining_sv2::OpenStandardMiningChannel,
};

use crate::{Instruction, Operation, Program, ProgramContext, Protocol};

pub type RoleId = usize;
pub type ConnectionId = usize;
pub type SessionId = usize;
pub type VariableIndex = usize;
/// A mining channel a program opened, identified by the order it was opened in.
pub type ChannelSlot = usize;

/// Where the `channel_id` or `job_id` of a mining message comes from.
///
/// A program can write an identifier down, or take the one the server assigned. Only the
/// second can name a channel that exists, and keeping the difference is the whole point of
/// binding channels at run time.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum IdSource {
    Literal(u32),
    /// The identifier the server gave the channel in this slot.
    ChannelId(ChannelSlot),
    /// The job the server announced on the channel in this slot.
    JobId(ChannelSlot),
}

/// The `OpenStandardMiningChannel` a program asked for.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ChannelOpenSpec {
    pub request_id: u32,
    pub user_identity: String,
    pub nominal_hash_rate: f32,
    pub max_target: [u8; 32],
}

/// A `SetupConnection` as the program describes it, before it is encoded.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SetupConnectionSpec {
    pub protocol: Protocol,
    pub min_version: u16,
    pub max_version: u16,
    pub flags: u32,
    pub endpoint_host: String,
    pub endpoint_port: u16,
    pub vendor: String,
    pub hardware_version: String,
    pub firmware: String,
    pub device_id: String,
}

impl Default for SetupConnectionSpec {
    fn default() -> Self {
        Self {
            protocol: Protocol::Mining,
            min_version: 2,
            max_version: 2,
            flags: 0,
            endpoint_host: "0.0.0.0".to_string(),
            endpoint_port: 0,
            vendor: "stratamoto".to_string(),
            hardware_version: String::new(),
            firmware: String::new(),
            device_id: String::new(),
        }
    }
}

/// A single step of a compiled program, ready to be executed against a deployment.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Open a transport from one role to another.
    Connect {
        connection: ConnectionId,
        role: RoleId,
    },
    /// Send one frame.
    Send {
        connection: ConnectionId,
        extension_type: u16,
        message_type: u8,
        channel_msg: bool,
        payload: Vec<u8>,
    },
    /// Await the answer to the last request and bind it to a session.
    AwaitSetupResponse {
        connection: ConnectionId,
        session: SessionId,
        protocol: Protocol,
        /// Whether the `SetupConnection` was the first message on its connection, which is
        /// the only one a server owes an answer to.
        first_on_connection: bool,
    },
    /// Open a standard mining channel, binding what the server answers to a slot.
    OpenStandardMiningChannel {
        connection: ConnectionId,
        channel: ChannelSlot,
        request_id: u32,
        payload: Vec<u8>,
        /// Whether the program had already put a frame of its own choosing on this
        /// connection. A server may drop a client that sends something it does not accept, so
        /// after one of those nothing it does or does not answer is owed.
        after_raw_frame: bool,
    },
    /// Submit a share against a job on a channel.
    ///
    /// Unlike the other sends this one is not encoded ahead of time: the identifiers it names
    /// belong to the server, so they are only known once it has answered.
    SubmitSharesStandard {
        connection: ConnectionId,
        channel_id: IdSource,
        sequence_number: u32,
        job_id: IdSource,
        nonce: u32,
        ntime: u32,
        version: u32,
    },
    AdvanceTime(Duration),
    /// Collect everything the deployment sent since the previous probe.
    Probe,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CompiledProgram {
    pub context: ProgramContext,
    pub actions: Vec<Action>,
    pub metadata: CompiledMetadata,
}

/// Links compiled actions back to the instructions and variables they came from, so that a
/// failure can be reported against the program that produced it.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CompiledMetadata {
    /// Instruction index each action was compiled from.
    pub action_instructions: Vec<usize>,
    /// Variable index that defines each session.
    pub session_variables: HashMap<SessionId, VariableIndex>,
    /// Variable index that defines each connection.
    pub connection_variables: HashMap<ConnectionId, VariableIndex>,
    /// The `SetupConnection` sent on each session.
    pub session_setups: HashMap<SessionId, SetupConnectionSpec>,
    /// The `OpenStandardMiningChannel` sent for each channel slot.
    pub channel_opens: HashMap<ChannelSlot, ChannelOpenSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CompilerError {
    /// An input referred to a variable the compiler never produced a value for.
    MissingValue(usize),
    /// An input had a value of the wrong shape, i.e. the program was not validated.
    UnexpectedValue(usize),
    /// A string field exceeded the 255 bytes `STR0_255` allows.
    StringTooLong(usize),
    Encoding(String),
}

impl fmt::Display for CompilerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompilerError::MissingValue(i) => write!(f, "v{i} has no value"),
            CompilerError::UnexpectedValue(i) => write!(f, "v{i} has the wrong type"),
            CompilerError::StringTooLong(i) => write!(f, "v{i} is longer than 255 bytes"),
            CompilerError::Encoding(e) => write!(f, "encoding failed: {e}"),
        }
    }
}

impl std::error::Error for CompilerError {}

#[derive(Debug, Clone)]
enum Value {
    Nop,
    Role(RoleId),
    Connection(ConnectionId),
    Session {
        connection: ConnectionId,
    },
    Channel(ChannelSlot),
    ChannelId(IdSource),
    JobId(IdSource),
    RequestId(u32),
    Hashrate(u32),
    Target([u8; 32]),
    SequenceNumber(u32),
    Nonce(u32),
    Ntime(u32),
    BlockVersion(u32),
    Version(u16),
    Flags(u32),
    Port(u16),
    Str(String),
    Bytes(Vec<u8>),
    Duration(Duration),
    SetupConnection(SetupConnectionSpec),
}

/// Lowers a [`Program`] to the actions that carry it out.
#[derive(Default)]
pub struct Compiler {
    values: Vec<Value>,
    actions: Vec<Action>,
    metadata: CompiledMetadata,
    connections: usize,
    sessions: usize,
    channels: usize,
    /// Connections something has already been sent on.
    sent: HashSet<ConnectionId>,
    /// Connections the program has put a frame of its own choosing on.
    raw_framed: HashSet<ConnectionId>,
}

impl Compiler {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn compile(mut self, program: &Program) -> Result<CompiledProgram, CompilerError> {
        // Connections that exist before the program runs are addressable via LoadConnection,
        // and are already set up, so nothing sent on them is a first message.
        self.connections = program.context.num_connections;
        self.sent.extend(0..program.context.num_connections);

        for (index, instruction) in program.instructions.iter().enumerate() {
            self.compile_instruction(index, instruction)?;
        }

        Ok(CompiledProgram {
            context: program.context.clone(),
            actions: self.actions,
            metadata: self.metadata,
        })
    }

    fn push_action(&mut self, instruction: usize, action: Action) {
        self.metadata.action_instructions.push(instruction);
        self.actions.push(action);
    }

    fn value(&self, index: usize) -> Result<&Value, CompilerError> {
        self.values
            .get(index)
            .ok_or(CompilerError::MissingValue(index))
    }

    fn compile_instruction(
        &mut self,
        index: usize,
        instruction: &Instruction,
    ) -> Result<(), CompilerError> {
        let inputs = &instruction.inputs;

        match &instruction.operation {
            Operation::Nop {
                outputs,
                inner_outputs,
            } => {
                for _ in 0..(outputs + inner_outputs) {
                    self.values.push(Value::Nop);
                }
            }

            Operation::LoadRole(i) => self.values.push(Value::Role(*i)),
            Operation::LoadConnection(i) => self.values.push(Value::Connection(*i)),
            Operation::LoadVersion(v) => self.values.push(Value::Version(*v)),
            Operation::LoadFlags(v) => self.values.push(Value::Flags(*v)),
            Operation::LoadPort(v) => self.values.push(Value::Port(*v)),
            Operation::LoadStr(v) => self.values.push(Value::Str(v.clone())),
            Operation::LoadBytes(v) => self.values.push(Value::Bytes(v.clone())),
            Operation::LoadDuration(v) => self.values.push(Value::Duration(*v)),

            Operation::LoadRequestId(v) => self.values.push(Value::RequestId(*v)),
            Operation::LoadHashrate(v) => self.values.push(Value::Hashrate(*v)),
            Operation::LoadTarget(v) => self.values.push(Value::Target(*v)),
            Operation::LoadChannelId(v) => {
                self.values.push(Value::ChannelId(IdSource::Literal(*v)));
            }
            Operation::LoadJobId(v) => self.values.push(Value::JobId(IdSource::Literal(*v))),
            Operation::LoadSequenceNumber(v) => self.values.push(Value::SequenceNumber(*v)),
            Operation::LoadNonce(v) => self.values.push(Value::Nonce(*v)),
            Operation::LoadNtime(v) => self.values.push(Value::Ntime(*v)),
            Operation::LoadBlockVersion(v) => self.values.push(Value::BlockVersion(*v)),

            Operation::OpenStandardMiningChannel => {
                let connection = self.session_connection(inputs[0])?;
                let request_id = self.request_id(inputs[1])?;
                let user_identity = self.string(inputs[2])?;
                let nominal_hash_rate = f32::from_bits(self.hashrate(inputs[3])?);
                let max_target = self.target(inputs[4])?;

                let spec = ChannelOpenSpec {
                    request_id,
                    user_identity,
                    nominal_hash_rate,
                    max_target,
                };
                let payload = encode_open_standard_mining_channel(&spec, inputs[2])?;

                let channel = self.channels;
                self.channels += 1;
                self.metadata.channel_opens.insert(channel, spec);
                self.values.push(Value::Channel(channel));
                self.sent.insert(connection);
                let after_raw_frame = self.raw_framed.contains(&connection);

                self.push_action(
                    index,
                    Action::OpenStandardMiningChannel {
                        connection,
                        channel,
                        request_id,
                        payload,
                        after_raw_frame,
                    },
                );
            }

            Operation::ChannelIdOf => {
                let channel = self.channel(inputs[0])?;
                self.values
                    .push(Value::ChannelId(IdSource::ChannelId(channel)));
            }

            Operation::JobIdOf => {
                let channel = self.channel(inputs[0])?;
                self.values.push(Value::JobId(IdSource::JobId(channel)));
            }

            Operation::SubmitSharesStandard => {
                let connection = self.session_connection(inputs[0])?;
                let channel_id = self.channel_id(inputs[1])?;
                let sequence_number = self.sequence_number(inputs[2])?;
                let job_id = self.job_id(inputs[3])?;
                let nonce = self.nonce(inputs[4])?;
                let ntime = self.ntime(inputs[5])?;
                let version = self.block_version(inputs[6])?;

                self.sent.insert(connection);
                self.push_action(
                    index,
                    Action::SubmitSharesStandard {
                        connection,
                        channel_id,
                        sequence_number,
                        job_id,
                        nonce,
                        ntime,
                        version,
                    },
                );
            }

            Operation::Connect => {
                let role = self.role(inputs[0])?;
                let connection = self.connections;
                self.connections += 1;

                self.metadata
                    .connection_variables
                    .insert(connection, self.values.len());
                self.values.push(Value::Connection(connection));
                self.push_action(
                    index,
                    Action::Connect { connection, role },
                );
            }

            Operation::BeginBuildSetupConnection => {
                self.values
                    .push(Value::SetupConnection(SetupConnectionSpec::default()));
            }

            Operation::SetVersions => {
                let min_version = self.version(inputs[1])?;
                let max_version = self.version(inputs[2])?;
                let spec = self.setup_mut(inputs[0])?;
                spec.min_version = min_version;
                spec.max_version = max_version;
            }

            Operation::SetFlags => {
                let flags = self.flags(inputs[1])?;
                let spec = self.setup_mut(inputs[0])?;
                spec.flags = flags;
            }

            Operation::SetEndpoint => {
                let host = self.string(inputs[1])?;
                let port = self.port(inputs[2])?;
                let spec = self.setup_mut(inputs[0])?;
                spec.endpoint_host = host;
                spec.endpoint_port = port;
            }

            Operation::SetDeviceInfo => {
                let vendor = self.string(inputs[1])?;
                let hardware_version = self.string(inputs[2])?;
                let firmware = self.string(inputs[3])?;
                let device_id = self.string(inputs[4])?;
                let spec = self.setup_mut(inputs[0])?;
                spec.vendor = vendor;
                spec.hardware_version = hardware_version;
                spec.firmware = firmware;
                spec.device_id = device_id;
            }

            Operation::EndBuildSetupConnection { protocol } => {
                let mut spec = self.setup(inputs[0])?;
                spec.protocol = *protocol;
                self.values.push(Value::SetupConnection(spec));
            }

            Operation::SendSetupConnection { protocol } => {
                let connection = self.connection(inputs[0])?;
                let first_on_connection = self.sent.insert(connection);
                let spec = self.setup(inputs[1])?;
                let payload = encode_setup_connection(&spec, inputs[1])?;

                let session = self.sessions;
                self.sessions += 1;
                self.metadata
                    .session_variables
                    .insert(session, self.values.len());
                self.metadata.session_setups.insert(session, spec);
                self.values.push(Value::Session { connection });

                self.push_action(
                    index,
                    Action::Send {
                        connection,
                        extension_type: 0,
                        message_type: MESSAGE_TYPE_SETUP_CONNECTION,
                        channel_msg: false,
                        payload,
                    },
                );
                self.push_action(
                    index,
                    Action::AwaitSetupResponse {
                        connection,
                        session,
                        protocol: *protocol,
                        first_on_connection,
                    },
                );
            }

            Operation::SendRawFrame {
                message_type,
                extension_type,
            } => {
                let connection = self.connection(inputs[0])?;
                let payload = self.bytes(inputs[1])?;
                self.sent.insert(connection);
                self.raw_framed.insert(connection);
                self.push_action(
                    index,
                    Action::Send {
                        connection,
                        extension_type: *extension_type,
                        message_type: *message_type,
                        channel_msg: false,
                        payload,
                    },
                );
            }

            Operation::AdvanceTime => {
                let duration = self.duration(inputs[0])?;
                self.push_action(index, Action::AdvanceTime(duration));
            }

            Operation::Probe => self.push_action(index, Action::Probe),
        }

        Ok(())
    }

    fn role(&self, i: usize) -> Result<RoleId, CompilerError> {
        match self.value(i)? {
            Value::Role(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn connection(&self, i: usize) -> Result<ConnectionId, CompilerError> {
        match self.value(i)? {
            Value::Connection(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn version(&self, i: usize) -> Result<u16, CompilerError> {
        match self.value(i)? {
            Value::Version(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn flags(&self, i: usize) -> Result<u32, CompilerError> {
        match self.value(i)? {
            Value::Flags(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn port(&self, i: usize) -> Result<u16, CompilerError> {
        match self.value(i)? {
            Value::Port(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn string(&self, i: usize) -> Result<String, CompilerError> {
        match self.value(i)? {
            Value::Str(v) => Ok(v.clone()),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn bytes(&self, i: usize) -> Result<Vec<u8>, CompilerError> {
        match self.value(i)? {
            Value::Bytes(v) => Ok(v.clone()),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn duration(&self, i: usize) -> Result<Duration, CompilerError> {
        match self.value(i)? {
            Value::Duration(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn session_connection(&self, i: usize) -> Result<ConnectionId, CompilerError> {
        match self.value(i)? {
            Value::Session { connection } => Ok(*connection),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn channel(&self, i: usize) -> Result<ChannelSlot, CompilerError> {
        match self.value(i)? {
            Value::Channel(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn channel_id(&self, i: usize) -> Result<IdSource, CompilerError> {
        match self.value(i)? {
            Value::ChannelId(v) => Ok(v.clone()),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn job_id(&self, i: usize) -> Result<IdSource, CompilerError> {
        match self.value(i)? {
            Value::JobId(v) => Ok(v.clone()),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn request_id(&self, i: usize) -> Result<u32, CompilerError> {
        match self.value(i)? {
            Value::RequestId(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn hashrate(&self, i: usize) -> Result<u32, CompilerError> {
        match self.value(i)? {
            Value::Hashrate(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn target(&self, i: usize) -> Result<[u8; 32], CompilerError> {
        match self.value(i)? {
            Value::Target(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn sequence_number(&self, i: usize) -> Result<u32, CompilerError> {
        match self.value(i)? {
            Value::SequenceNumber(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn nonce(&self, i: usize) -> Result<u32, CompilerError> {
        match self.value(i)? {
            Value::Nonce(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn ntime(&self, i: usize) -> Result<u32, CompilerError> {
        match self.value(i)? {
            Value::Ntime(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn block_version(&self, i: usize) -> Result<u32, CompilerError> {
        match self.value(i)? {
            Value::BlockVersion(v) => Ok(*v),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn setup(&self, i: usize) -> Result<SetupConnectionSpec, CompilerError> {
        match self.value(i)? {
            Value::SetupConnection(v) => Ok(v.clone()),
            _ => Err(CompilerError::UnexpectedValue(i)),
        }
    }

    fn setup_mut(&mut self, i: usize) -> Result<&mut SetupConnectionSpec, CompilerError> {
        match self.values.get_mut(i) {
            Some(Value::SetupConnection(v)) => Ok(v),
            Some(_) => Err(CompilerError::UnexpectedValue(i)),
            None => Err(CompilerError::MissingValue(i)),
        }
    }
}

fn encode_setup_connection(
    spec: &SetupConnectionSpec,
    variable: usize,
) -> Result<Vec<u8>, CompilerError> {
    let message = SetupConnection {
        protocol: spec
            .protocol
            .id()
            .try_into()
            .expect("Protocol only holds ids the wire type accepts"),
        min_version: spec.min_version,
        max_version: spec.max_version,
        flags: spec.flags,
        endpoint_host: str0_255(&spec.endpoint_host, variable)?,
        endpoint_port: spec.endpoint_port,
        vendor: str0_255(&spec.vendor, variable)?,
        hardware_version: str0_255(&spec.hardware_version, variable)?,
        firmware: str0_255(&spec.firmware, variable)?,
        device_id: str0_255(&spec.device_id, variable)?,
    };

    encode(message)
}

fn encode<T: Sv2Serialize + GetSize>(message: T) -> Result<Vec<u8>, CompilerError> {
    let mut payload = vec![0u8; message.get_size()];
    to_writer(message, &mut payload).map_err(|e| CompilerError::Encoding(format!("{e:?}")))?;
    Ok(payload)
}

fn str0_255(value: &str, variable: usize) -> Result<Str0255<'_>, CompilerError> {
    value
        .try_into()
        .map_err(|_| CompilerError::StringTooLong(variable))
}

fn encode_open_standard_mining_channel(
    spec: &ChannelOpenSpec,
    variable: usize,
) -> Result<Vec<u8>, CompilerError> {
    let message = OpenStandardMiningChannel {
        request_id: spec.request_id,
        user_identity: str0_255(&spec.user_identity, variable)?,
        nominal_hash_rate: spec.nominal_hash_rate,
        max_target: (&spec.max_target).into(),
    };
    encode(message)
}
