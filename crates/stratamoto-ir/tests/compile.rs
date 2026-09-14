use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::{Action, Compiler},
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
    let setup = one(builder.append_op(Operation::BeginBuildSetupConnection, &[]).unwrap());
    let setup = one(
        builder
            .append_op(Operation::EndBuildSetupConnection { protocol }, &[&setup])
            .unwrap(),
    );
    builder
        .append_op(Operation::SendSetupConnection { protocol }, &[connection, &setup])
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
    let bytes = one(builder.append_op(Operation::LoadBytes(vec![0]), &[]).unwrap());
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

    assert_eq!(first_on_connection(&builder.finalize().unwrap()), vec![false]);
}
