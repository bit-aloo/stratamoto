use stratamoto::{
    oracle::{CrashOracle, Oracle, OracleResult, SetupConnectionOracle},
    runner::{self, Execution, SetupResponse},
    transport::Deployment,
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::Compiler,
};
use stratamoto_targets::pool::PoolDeployment;

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

fn context() -> ProgramContext {
    ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    }
}

fn setup_connection(protocol: Protocol, min_version: u16, max_version: u16, flags: u32) -> Program {
    let mut builder = ProgramBuilder::new(context());
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

/// Run a program against the pool, checking both oracles, and hand back the execution.
fn run_checked(deployment: &PoolDeployment, program: &Program) -> Execution {
    let compiled = Compiler::new().compile(program).unwrap();
    let execution = runner::run(deployment, &compiled);

    for result in [
        SetupConnectionOracle.evaluate(deployment, &compiled, &execution),
        CrashOracle.evaluate(deployment, &compiled, &execution),
    ] {
        if let OracleResult::Fail(reason) = result {
            panic!("{reason}");
        }
    }
    execution
}

fn answer(deployment: &PoolDeployment, program: &Program) -> SetupResponse {
    run_checked(deployment, program).sessions[&0].response.clone()
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

/// A found bug in sv2-apps' pool at the revision this harness tracks, kept as a runnable
/// record. A single downstream sends a rejected SetupConnection and then a second frame on the
/// same connection; the pool responds by cancelling its global shutdown token, which stops it
/// serving every other downstream and its template receiver too. A rejected SetupConnection
/// alone does not do this, so it is the second frame on a connection being torn down that
/// escalates a per connection disconnect into a whole role shutdown.
///
/// Ignored because it depends on the bug being present: if sv2-apps fixes it, the pool stays
/// alive and this fails, which is the signal to update the record. Run with
/// `cargo test -p stratamoto-targets --test pool -- --ignored`.
#[test]
#[ignore = "documents an unfixed sv2-apps pool DoS"]
fn a_second_setup_on_one_connection_shuts_the_whole_pool_down() {
    let deployment = PoolDeployment::start().expect("the pool starts against the harness' TP");
    assert!(deployment.is_alive(), "the pool serves before the program");

    let mut builder = ProgramBuilder::new(context());
    let role = one(builder.append_op(Operation::LoadRole(0), &[]).unwrap());
    let connection = one(builder.append_op(Operation::Connect, &[&role]).unwrap());
    for protocol in [Protocol::TemplateDistribution, Protocol::Mining] {
        let setup = one(builder.append_op(Operation::BeginBuildSetupConnection, &[]).unwrap());
        let setup = one(
            builder
                .append_op(Operation::EndBuildSetupConnection { protocol }, &[&setup])
                .unwrap(),
        );
        builder
            .append_op(Operation::SendSetupConnection { protocol }, &[&connection, &setup])
            .unwrap();
    }
    let program = Compiler::new().compile(&builder.finalize().unwrap()).unwrap();

    let _ = runner::run(&deployment, &program);

    // The pool has cancelled its global token; it no longer answers a fresh connection.
    assert!(
        !deployment.is_alive(),
        "the pool still serves, so the DoS may have been fixed upstream"
    );
}
