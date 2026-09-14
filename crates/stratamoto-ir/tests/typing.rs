use stratamoto_ir::{
    Instruction, Operation, ProgramBuilder, ProgramContext, Protocol, Variable,
    errors::ProgramValidationError,
};

fn context() -> ProgramContext {
    ProgramContext {
        num_roles: 2,
        num_connections: 0,
        seed: 0,
    }
}

/// Build a connection and a finalized `SetupConnection` for `protocol`.
fn connection_and_setup(builder: &mut ProgramBuilder, protocol: Protocol) -> (usize, usize) {
    let role = builder
        .append(Instruction::new(Operation::LoadRole(0), vec![]))
        .unwrap()[0]
        .index;
    let connection = builder
        .append(Instruction::new(Operation::Connect, vec![role]))
        .unwrap()[0]
        .index;
    let mutable = builder
        .append(Instruction::new(
            Operation::BeginBuildSetupConnection,
            vec![],
        ))
        .unwrap()[0]
        .index;
    let setup = builder
        .append(Instruction::new(
            Operation::EndBuildSetupConnection { protocol },
            vec![mutable],
        ))
        .unwrap()[0]
        .index;
    (connection, setup)
}

#[test]
fn setup_connection_for_one_protocol_cannot_open_a_session_for_another() {
    let mut builder = ProgramBuilder::new(context());
    let (connection, setup) = connection_and_setup(&mut builder, Protocol::TemplateDistribution);

    let error = builder
        .append(Instruction::new(
            Operation::SendSetupConnection {
                protocol: Protocol::Mining,
            },
            vec![connection, setup],
        ))
        .unwrap_err();

    assert_eq!(
        error,
        ProgramValidationError::InvalidVariableType {
            is: Variable::ConstSetupConnection(Protocol::TemplateDistribution),
            expected: Variable::ConstSetupConnection(Protocol::Mining),
        }
    );
}

#[test]
fn a_matching_protocol_opens_a_session_of_that_protocol() {
    let mut builder = ProgramBuilder::new(context());
    let (connection, setup) = connection_and_setup(&mut builder, Protocol::Mining);

    let outputs = builder
        .append(Instruction::new(
            Operation::SendSetupConnection {
                protocol: Protocol::Mining,
            },
            vec![connection, setup],
        ))
        .unwrap();

    assert_eq!(outputs[0].var, Variable::Session(Protocol::Mining));
}

#[test]
fn a_setup_connection_cannot_be_edited_once_finalized() {
    let mut builder = ProgramBuilder::new(context());
    let mutable = builder
        .append(Instruction::new(
            Operation::BeginBuildSetupConnection,
            vec![],
        ))
        .unwrap()[0]
        .index;
    builder
        .append(Instruction::new(
            Operation::EndBuildSetupConnection {
                protocol: Protocol::Mining,
            },
            vec![mutable],
        ))
        .unwrap();

    let flags = builder
        .append(Instruction::new(Operation::LoadFlags(1), vec![]))
        .unwrap()[0]
        .index;

    assert_eq!(
        builder
            .append(Instruction::new(Operation::SetFlags, vec![mutable, flags]))
            .unwrap_err(),
        ProgramValidationError::VariableNotDefined(mutable)
    );
}

#[test]
fn a_role_outside_the_context_is_rejected() {
    let mut builder = ProgramBuilder::new(context());
    assert_eq!(
        builder
            .append(Instruction::new(Operation::LoadRole(2), vec![]))
            .unwrap_err(),
        ProgramValidationError::RoleNotFound(2)
    );
}

#[test]
fn an_unclosed_block_is_rejected() {
    let mut builder = ProgramBuilder::new(context());
    builder
        .append(Instruction::new(
            Operation::BeginBuildSetupConnection,
            vec![],
        ))
        .unwrap();

    assert_eq!(
        builder.finalize().unwrap_err(),
        ProgramValidationError::UnfinishedBlock(Operation::BeginBuildSetupConnection)
    );
}

#[test]
fn flags_cannot_be_used_where_a_version_is_expected() {
    let mut builder = ProgramBuilder::new(context());
    let mutable = builder
        .append(Instruction::new(
            Operation::BeginBuildSetupConnection,
            vec![],
        ))
        .unwrap()[0]
        .index;
    let version = builder
        .append(Instruction::new(Operation::LoadVersion(2), vec![]))
        .unwrap()[0]
        .index;
    let flags = builder
        .append(Instruction::new(Operation::LoadFlags(0), vec![]))
        .unwrap()[0]
        .index;

    assert_eq!(
        builder
            .append(Instruction::new(
                Operation::SetVersions,
                vec![mutable, version, flags]
            ))
            .unwrap_err(),
        ProgramValidationError::InvalidVariableType {
            is: Variable::Flags,
            expected: Variable::Version,
        }
    );
}

#[test]
fn nopping_an_unused_instruction_keeps_the_program_valid() {
    let mut builder = ProgramBuilder::new(context());
    let (connection, setup) = connection_and_setup(&mut builder, Protocol::Mining);

    // A load nothing consumes, sitting between the setup and the send.
    let unused = builder
        .append(Instruction::new(Operation::LoadFlags(7), vec![]))
        .unwrap()[0]
        .index;

    builder
        .append(Instruction::new(
            Operation::SendSetupConnection {
                protocol: Protocol::Mining,
            },
            vec![connection, setup],
        ))
        .unwrap();
    let mut program = builder.finalize().unwrap();
    assert!(program.is_statically_valid());

    let index = program
        .instructions
        .iter()
        .position(|i| matches!(i.operation, Operation::LoadFlags(_)))
        .unwrap();
    program.instructions[index].nop();
    assert!(program.is_statically_valid());

    // Dropping the nop renumbers the inputs of every later instruction.
    program.remove_nops();
    assert!(program.is_statically_valid());
    assert!(
        !program
            .instructions
            .iter()
            .any(|i| matches!(i.operation, Operation::Nop { .. }))
    );

    let send = program.instructions.last().unwrap();
    assert_eq!(send.inputs, vec![connection, setup]);
    assert!(unused > setup);
}
