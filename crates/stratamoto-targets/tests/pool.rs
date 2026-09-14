use stratamoto::{
    oracle::{Oracle, OracleResult, SetupConnectionOracle},
    runner::{self, SetupResponse},
};
use stratamoto_ir::{
    Operation, Program, ProgramBuilder, ProgramContext, Protocol, compiler::Compiler,
};
use stratamoto_targets::pool::PoolDeployment;

fn setup_connection(protocol: Protocol, min_version: u16, max_version: u16, flags: u32) -> Program {
    let mut builder = ProgramBuilder::new(ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    });
    let one = |mut variables: Vec<_>| -> stratamoto_ir::IndexedVariable { variables.remove(0) };

    let role = one(builder.append_op(Operation::LoadRole(0), &[]).unwrap());
    let connection = one(builder.append_op(Operation::Connect, &[&role]).unwrap());
    let min_version = one(builder.append_op(Operation::LoadVersion(min_version), &[]).unwrap());
    let max_version = one(builder.append_op(Operation::LoadVersion(max_version), &[]).unwrap());
    let flags = one(builder.append_op(Operation::LoadFlags(flags), &[]).unwrap());
    let setup = one(builder.append_op(Operation::BeginBuildSetupConnection, &[]).unwrap());
    builder
        .append_op(Operation::SetVersions, &[&setup, &min_version, &max_version])
        .unwrap();
    builder.append_op(Operation::SetFlags, &[&setup, &flags]).unwrap();
    let setup = one(
        builder
            .append_op(Operation::EndBuildSetupConnection { protocol }, &[&setup])
            .unwrap(),
    );
    builder
        .append_op(Operation::SendSetupConnection { protocol }, &[&connection, &setup])
        .unwrap();
    builder.finalize().unwrap()
}

/// Run a program against the pool and hand back the one answer it drew, having checked the
/// answer is one the oracle accepts.
fn answer(deployment: &PoolDeployment, program: &Program) -> SetupResponse {
    let compiled = Compiler::new().compile(program).unwrap();
    let execution = runner::run(deployment, &compiled);

    if let OracleResult::Fail(reason) =
        SetupConnectionOracle.evaluate(deployment, &compiled, &execution)
    {
        panic!("{reason}");
    }
    execution.sessions[&0].response.clone()
}

/// One pool for every case: starting one waits on its first template, which takes seconds.
#[test]
fn the_real_pool_answers_setup_connection_within_the_specification() {
    let deployment = PoolDeployment::start().expect("the pool starts against the harness' TP");

    assert_eq!(
        answer(&deployment, &setup_connection(Protocol::Mining, 2, 2, 0)),
        SetupResponse::Success {
            used_version: 2,
            flags: 0
        }
    );

    // A work selection request is answered with REQUIRES_EXTENDED_CHANNELS.
    assert_eq!(
        answer(&deployment, &setup_connection(Protocol::Mining, 2, 2, 1 << 1)),
        SetupResponse::Success {
            used_version: 2,
            flags: 1 << 1
        }
    );

    assert!(matches!(
        answer(
            &deployment,
            &setup_connection(Protocol::TemplateDistribution, 2, 2, 0)
        ),
        SetupResponse::Error { .. }
    ));

    assert!(matches!(
        answer(&deployment, &setup_connection(Protocol::Mining, 3, 3, 0)),
        SetupResponse::Error { .. }
    ));
}
