use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::{Action, ClientViolation, Compiler},
};

fn builder() -> ProgramBuilder {
    ProgramBuilder::new(ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    })
}

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

fn connect(builder: &mut ProgramBuilder) -> IndexedVariable {
    let role = one(builder.append_op(Operation::LoadRole(0), &[]).unwrap());
    one(builder.append_op(Operation::Connect, &[&role]).unwrap())
}

fn send_setup(builder: &mut ProgramBuilder, connection: &IndexedVariable) {
    let protocol = Protocol::Mining;
    let setup = one(builder
        .append_op(Operation::BeginBuildSetupConnection, &[])
        .unwrap());
    let setup = one(builder
        .append_op(Operation::EndBuildSetupConnection { protocol }, &[&setup])
        .unwrap());
    builder
        .append_op(
            Operation::SendSetupConnection { protocol },
            &[connection, &setup],
        )
        .unwrap();
}

/// Whether each setup the program sends was the first message on its connection, in order.
fn first_on_connection(program: &Program) -> Vec<bool> {
    Compiler::new()
        .compile(program)
        .unwrap()
        .actions
        .iter()
        .filter_map(|action| match action {
            Action::AwaitSetupResponse {
                first_on_connection,
                ..
            } => Some(*first_on_connection),
            _ => None,
        })
        .collect()
}

#[test]
fn only_the_first_setup_on_a_connection_is_first() {
    let mut builder = builder();
    let connection = connect(&mut builder);
    send_setup(&mut builder, &connection);
    send_setup(&mut builder, &connection);

    assert_eq!(
        first_on_connection(&builder.finalize().unwrap()),
        vec![true, false]
    );
}

#[test]
fn every_new_connection_starts_over() {
    let mut builder = builder();
    let first = connect(&mut builder);
    let second = connect(&mut builder);
    send_setup(&mut builder, &first);
    send_setup(&mut builder, &second);

    assert_eq!(
        first_on_connection(&builder.finalize().unwrap()),
        vec![true, true]
    );
}

#[test]
fn a_raw_frame_sent_before_a_setup_makes_it_not_first() {
    let mut builder = builder();
    let connection = connect(&mut builder);
    let bytes = one(builder
        .append_op(Operation::LoadBytes(vec![0]), &[])
        .unwrap());
    builder
        .append_op(
            Operation::SendRawFrame {
                message_type: 0x10,
                extension_type: 0,
            },
            &[&connection, &bytes],
        )
        .unwrap();
    send_setup(&mut builder, &connection);

    assert_eq!(
        first_on_connection(&builder.finalize().unwrap()),
        vec![false]
    );
}

/// A raw frame on `connection` inside the success block of `attempt`.
fn raw_in_block(
    builder: &mut ProgramBuilder,
    connection: &IndexedVariable,
    attempt: &IndexedVariable,
) {
    builder
        .append_op(
            Operation::BeginOnSetupSuccess {
                protocol: Protocol::Mining,
            },
            &[attempt],
        )
        .unwrap();
    let bytes = one(builder
        .append_op(Operation::LoadBytes(vec![0]), &[])
        .unwrap());
    builder
        .append_op(
            Operation::SendRawFrame {
                message_type: 0x7f,
                extension_type: 0,
            },
            &[connection, &bytes],
        )
        .unwrap();
    builder
        .append_op(Operation::EndOnSetupSuccess, &[])
        .unwrap();
}

fn attempt(builder: &mut ProgramBuilder, connection: &IndexedVariable) -> IndexedVariable {
    let protocol = Protocol::Mining;
    let setup = one(builder
        .append_op(Operation::BeginBuildSetupConnection, &[])
        .unwrap());
    let setup = one(builder
        .append_op(Operation::EndBuildSetupConnection { protocol }, &[&setup])
        .unwrap());
    one(builder
        .append_op(
            Operation::SendSetupConnection { protocol },
            &[connection, &setup],
        )
        .unwrap())
}

/// A success block lowers to an enter that knows where its exit is, so the runner can skip
/// the whole of it in one step.
#[test]
fn a_success_block_knows_its_end() {
    let mut builder = builder();
    let connection = connect(&mut builder);
    let attempt = attempt(&mut builder, &connection);
    raw_in_block(&mut builder, &connection, &attempt);
    let compiled = Compiler::new()
        .compile(&builder.finalize().unwrap())
        .unwrap();

    let enter = compiled
        .actions
        .iter()
        .position(|a| matches!(a, Action::EnterOnSetupSuccess { .. }))
        .unwrap();
    let exit = compiled
        .actions
        .iter()
        .position(|a| matches!(a, Action::ExitBlock))
        .unwrap();
    assert_eq!(
        compiled.actions[enter],
        Action::EnterOnSetupSuccess {
            session: 0,
            end: exit
        }
    );
    assert!(matches!(compiled.actions[enter + 1], Action::Send { .. }));
    assert!(
        compiled.metadata.violations.is_empty(),
        "a valid program violates nothing"
    );
}

/// A setup that is not the first message on its connection is the client's violation, and so
/// is one after a frame of the program's own choosing; both are classified on the action.
#[test]
fn client_violations_are_attached_to_the_action() {
    let mut builder = builder();
    let connection = connect(&mut builder);
    send_setup(&mut builder, &connection);
    send_setup(&mut builder, &connection);
    let compiled = Compiler::new()
        .compile(&builder.finalize().unwrap())
        .unwrap();
    let awaits: Vec<usize> = compiled
        .actions
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a, Action::AwaitSetupResponse { .. }))
        .map(|(i, _)| i)
        .collect();
    assert!(!compiled.metadata.violations.contains_key(&awaits[0]));
    assert_eq!(
        compiled.metadata.violations[&awaits[1]],
        vec![ClientViolation::SetupNotFirst]
    );

    let mut second = crate::builder();
    let connection = connect(&mut second);
    let bytes = one(second
        .append_op(Operation::LoadBytes(vec![0]), &[])
        .unwrap());
    second
        .append_op(
            Operation::SendRawFrame {
                message_type: 0x7f,
                extension_type: 0,
            },
            &[&connection, &bytes],
        )
        .unwrap();
    send_setup(&mut second, &connection);
    let compiled = Compiler::new()
        .compile(&second.finalize().unwrap())
        .unwrap();
    let await_ = compiled
        .actions
        .iter()
        .position(|a| matches!(a, Action::AwaitSetupResponse { .. }))
        .unwrap();
    assert_eq!(
        compiled.metadata.violations[&await_],
        vec![
            ClientViolation::SetupNotFirst,
            ClientViolation::RawFrameBefore
        ]
    );
}
