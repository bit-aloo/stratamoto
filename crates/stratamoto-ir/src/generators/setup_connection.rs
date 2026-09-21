use rand::{RngExt, seq::IndexedRandom};

use crate::{
    IndexedVariable, Operation, ProgramBuilder, Protocol, Variable, errors::ProgramValidationError,
    generators::Generator,
};

const PROTOCOLS: [Protocol; 3] = [
    Protocol::Mining,
    Protocol::JobDeclaration,
    Protocol::TemplateDistribution,
];

/// The one version the specification defines.
pub const CURRENT_VERSION: u16 = 2;

/// The `SetupConnection.flags` bits a subprotocol defines (sections 5.1 and 6.1); everything
/// else is a bit no server has a meaning for.
#[must_use]
pub const fn defined_flags(protocol: Protocol) -> u32 {
    match protocol {
        // REQUIRES_STANDARD_JOBS, REQUIRES_WORK_SELECTION, REQUIRES_VERSION_ROLLING.
        Protocol::Mining => 0b111,
        // REQUIRES_ASYNC_JOB_MINING.
        Protocol::JobDeclaration => 0b1,
        Protocol::TemplateDistribution => 0,
    }
}

/// Which connection a fragment goes on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionChoice {
    /// Open a new one. A setup is then the first message on it, which is the only setup a
    /// server owes an answer to, so this is what reaches deep states.
    Fresh,
    /// One the program already has in scope, opening a new one only if there is none. A setup
    /// on it is a second one, which the protocol forbids.
    Reuse,
    /// Either, at random.
    Any,
}

/// Opens a connection between two roles and sets it up for a subprotocol, as a conforming
/// client would: the current version, only flags the subprotocol defines, and a fresh
/// connection unless told otherwise.
///
/// The attempt it produces is what a success block turns into the session later subprotocol
/// messages consume, so this is the fragment every mining, job declaration or template
/// distribution program starts from. What a conforming client would not do is the
/// [`AdversarialSetupGenerator`]'s business.
pub struct SetupConnectionGenerator {
    /// The role the connection is opened to.
    pub role: usize,
    /// The subprotocol to set the connection up for, or any of them when unset.
    pub protocol: Option<Protocol>,
    pub connection: ConnectionChoice,
}

impl SetupConnectionGenerator {
    /// A valid setup for any protocol on a fresh connection to `role`.
    #[must_use]
    pub fn new(role: usize) -> Self {
        Self {
            role,
            protocol: None,
            connection: ConnectionChoice::Fresh,
        }
    }
}

impl<R: RngExt> Generator<R> for SetupConnectionGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), ProgramValidationError> {
        let protocol = self
            .protocol
            .unwrap_or_else(|| *PROTOCOLS.choose(rng).expect("PROTOCOLS is not empty"));
        let connection = connection(builder, rng, self.role, self.connection)?;

        // Any subset of the defined flags is a request a server has a meaning for.
        let flags = rng.random::<u32>() & defined_flags(protocol);
        setup(
            builder,
            rng,
            &connection,
            protocol,
            CURRENT_VERSION,
            CURRENT_VERSION,
            flags,
        )?;
        Ok(())
    }
}

/// One thing a conforming client does not do, done on purpose.
///
/// The first three are requests a server must reject, so they reach its error paths; the
/// other two are violations of the protocol's order, which the compiler classifies on the
/// actions so that an oracle relaxes what the server owes in answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupViolation {
    /// `min_version` above `max_version`, so no version can be agreed.
    EmptyVersionRange,
    /// A version range without the one version that exists.
    UnknownVersion,
    /// A flag bit the subprotocol does not define.
    UndefinedFlags,
    /// A frame of the program's own choosing before the setup, so the setup is not the first
    /// message on its connection.
    AfterRawFrame,
    /// A second setup on a connection that already has one.
    SecondSetup,
}

impl SetupViolation {
    pub const ALL: [SetupViolation; 5] = [
        SetupViolation::EmptyVersionRange,
        SetupViolation::UnknownVersion,
        SetupViolation::UndefinedFlags,
        SetupViolation::AfterRawFrame,
        SetupViolation::SecondSetup,
    ];

    pub fn any<R: RngExt>(rng: &mut R) -> Self {
        *Self::ALL.choose(rng).expect("ALL is not empty")
    }
}

/// A setup that violates exactly one named constraint, and is otherwise what a conforming
/// client would send.
pub struct AdversarialSetupGenerator {
    pub role: usize,
    pub protocol: Option<Protocol>,
    pub violation: SetupViolation,
}

impl<R: RngExt> Generator<R> for AdversarialSetupGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), ProgramValidationError> {
        let protocol = self
            .protocol
            .unwrap_or_else(|| *PROTOCOLS.choose(rng).expect("PROTOCOLS is not empty"));
        let valid_flags = rng.random::<u32>() & defined_flags(protocol);
        let (min, max, flags) = (CURRENT_VERSION, CURRENT_VERSION, valid_flags);

        match self.violation {
            SetupViolation::EmptyVersionRange => {
                let connection = connection(builder, rng, self.role, ConnectionChoice::Fresh)?;
                setup(builder, rng, &connection, protocol, max + 1, min, flags)?;
            }
            SetupViolation::UnknownVersion => {
                let connection = connection(builder, rng, self.role, ConnectionChoice::Fresh)?;
                let version = *[0, 1, CURRENT_VERSION + 1, u16::MAX]
                    .choose(rng)
                    .expect("not empty");
                setup(builder, rng, &connection, protocol, version, version, flags)?;
            }
            SetupViolation::UndefinedFlags => {
                let connection = connection(builder, rng, self.role, ConnectionChoice::Fresh)?;
                // Some bits outside the definition, and at least one whatever the draw.
                let undefined = !defined_flags(protocol);
                let mut flags = flags | (rng.random::<u32>() & undefined);
                if flags & undefined == 0 {
                    flags |= undefined.isolate_lowest_one();
                }
                setup(builder, rng, &connection, protocol, min, max, flags)?;
            }
            SetupViolation::AfterRawFrame => {
                let connection = connection(builder, rng, self.role, ConnectionChoice::Fresh)?;
                let bytes = one(builder.append_op(Operation::LoadBytes(vec![0; 6]), &[])?);
                builder.append_op(
                    Operation::SendRawFrame {
                        message_type: rng.random(),
                        extension_type: 0,
                    },
                    &[&connection, &bytes],
                )?;
                setup(builder, rng, &connection, protocol, min, max, flags)?;
            }
            SetupViolation::SecondSetup => {
                let connection = connection(builder, rng, self.role, ConnectionChoice::Fresh)?;
                setup(builder, rng, &connection, protocol, min, max, flags)?;
                setup(builder, rng, &connection, protocol, min, max, flags)?;
            }
        }
        Ok(())
    }
}

/// The connection a fragment goes on, opened if the choice or the program's state requires.
fn connection<R: RngExt>(
    builder: &mut ProgramBuilder,
    rng: &mut R,
    role: usize,
    choice: ConnectionChoice,
) -> Result<IndexedVariable, ProgramValidationError> {
    let reuse = match choice {
        ConnectionChoice::Fresh => None,
        ConnectionChoice::Reuse => builder.get_random_variable(rng, &Variable::Connection),
        ConnectionChoice::Any => {
            if rng.random_bool(0.5) {
                builder.get_random_variable(rng, &Variable::Connection)
            } else {
                None
            }
        }
    };
    match reuse {
        Some(connection) => Ok(connection),
        None => {
            let role = one(builder.append_op(Operation::LoadRole(role), &[])?);
            Ok(one(builder.append_op(Operation::Connect, &[&role])?))
        }
    }
}

/// Build and send a `SetupConnection` with the given versions and flags, sometimes with the
/// endpoint and device fields set as well, so that both shapes are in the corpus. Those start
/// at the values a client would send, and are worth setting at all because a field no
/// operation writes is a field no mutation can ever reach.
fn setup<R: RngExt>(
    builder: &mut ProgramBuilder,
    rng: &mut R,
    connection: &IndexedVariable,
    protocol: Protocol,
    min_version: u16,
    max_version: u16,
    flags: u32,
) -> Result<IndexedVariable, ProgramValidationError> {
    let min_version = one(builder.append_op(Operation::LoadVersion(min_version), &[])?);
    let max_version = one(builder.append_op(Operation::LoadVersion(max_version), &[])?);
    let flags = one(builder.append_op(Operation::LoadFlags(flags), &[])?);

    let setup = one(builder.append_op(Operation::BeginBuildSetupConnection, &[])?);
    builder.append_op(
        Operation::SetVersions,
        &[&setup, &min_version, &max_version],
    )?;
    builder.append_op(Operation::SetFlags, &[&setup, &flags])?;

    if rng.random_bool(0.5) {
        let host = one(builder.append_op(Operation::LoadStr("0.0.0.0".to_string()), &[])?);
        let port = one(builder.append_op(Operation::LoadPort(0), &[])?);
        builder.append_op(Operation::SetEndpoint, &[&setup, &host, &port])?;
    }

    if rng.random_bool(0.5) {
        let vendor = one(builder.append_op(Operation::LoadStr("stratamoto".to_string()), &[])?);
        let hardware = one(builder.append_op(Operation::LoadStr(String::new()), &[])?);
        let firmware = one(builder.append_op(Operation::LoadStr(String::new()), &[])?);
        let device = one(builder.append_op(Operation::LoadStr(String::new()), &[])?);
        builder.append_op(
            Operation::SetDeviceInfo,
            &[&setup, &vendor, &hardware, &firmware, &device],
        )?;
    }
    let setup = one(builder.append_op(Operation::EndBuildSetupConnection { protocol }, &[&setup])?);

    Ok(one(builder.append_op(
        Operation::SendSetupConnection { protocol },
        &[connection, &setup],
    )?))
}

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    assert_eq!(
        variables.len(),
        1,
        "operation produces exactly one variable"
    );
    variables.pop().expect("checked above")
}
