//! An oracle compares what the program asked for with what the run recorded, so a record that
//! goes missing is a failure and not a pass, and a stimulus the client got wrong relaxes the
//! server's obligation.

use stratamoto::{
    deployment::SimulatedDeployment,
    oracle::{HarnessIntegrityOracle, Oracle, OracleResult, SetupConnectionOracle},
    roles::RoleConfig,
    runner::{self, ActionOutcome, Execution},
    stratum_core::common_messages_sv2::Protocol as WireProtocol,
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::{Action, CompiledProgram, Compiler},
};

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

fn deployment() -> SimulatedDeployment {
    SimulatedDeployment::new(1, vec![RoleConfig::new(WireProtocol::MiningProtocol)])
}

/// `setups` mining setups on one connection, with a raw frame before the first when asked.
fn program(setups: usize, raw_frame_first: bool) -> Program {
    let mut b = ProgramBuilder::new(ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    });
    let role = one(b.append_op(Operation::LoadRole(0), &[]).unwrap());
    let connection = one(b.append_op(Operation::Connect, &[&role]).unwrap());
    if raw_frame_first {
        let bytes = one(b.append_op(Operation::LoadBytes(vec![0; 4]), &[]).unwrap());
        b.append_op(
            Operation::SendRawFrame {
                message_type: 0x7f,
                extension_type: 0,
            },
            &[&connection, &bytes],
        )
        .unwrap();
    }
    for _ in 0..setups {
        let setup = one(b
            .append_op(Operation::BeginBuildSetupConnection, &[])
            .unwrap());
        let setup = one(b
            .append_op(
                Operation::EndBuildSetupConnection {
                    protocol: Protocol::Mining,
                },
                &[&setup],
            )
            .unwrap());
        b.append_op(
            Operation::SendSetupConnection {
                protocol: Protocol::Mining,
            },
            &[&connection, &setup],
        )
        .unwrap();
    }
    b.finalize().unwrap()
}

fn run(program: &Program) -> (SimulatedDeployment, CompiledProgram, Execution) {
    let deployment = deployment();
    let compiled = Compiler::new().compile(program).unwrap();
    let execution = runner::run(&deployment, &compiled);
    (deployment, compiled, execution)
}

fn fails<O: Oracle>(
    oracle: O,
    deployment: &SimulatedDeployment,
    compiled: &CompiledProgram,
    execution: &Execution,
) -> Option<String> {
    match oracle.evaluate(deployment, compiled, execution) {
        OracleResult::Pass => None,
        OracleResult::Fail(reason) => Some(reason),
    }
}

fn awaits(compiled: &CompiledProgram) -> Vec<usize> {
    compiled
        .actions
        .iter()
        .enumerate()
        .filter(|(_, a)| matches!(a, Action::AwaitSetupResponse { .. }))
        .map(|(i, _)| i)
        .collect()
}

/// The mock answers every setup, so a valid program passes; and an owed setup with its
/// answer taken away fails.
#[test]
fn an_owed_setup_left_unanswered_fails() {
    let (deployment, compiled, execution) = run(&program(1, false));
    assert!(fails(SetupConnectionOracle, &deployment, &compiled, &execution).is_none());

    let mut unanswered = execution.clone();
    unanswered.outcomes[awaits(&compiled)[0]] = ActionOutcome::TimedOut;
    let reason = fails(SetupConnectionOracle, &deployment, &compiled, &unanswered).unwrap();
    assert!(reason.contains("MUST respond"), "{reason}");
}

/// A second setup on a connection is the client's violation: taking its answer away is no
/// failure of the server.
#[test]
fn an_unanswered_second_setup_is_the_clients_doing() {
    let (deployment, compiled, execution) = run(&program(2, false));
    let mut unanswered = execution.clone();
    unanswered.outcomes[awaits(&compiled)[1]] = ActionOutcome::TimedOut;
    assert!(fails(SetupConnectionOracle, &deployment, &compiled, &unanswered).is_none());
}

/// So is a setup after a frame of the program's own choosing.
#[test]
fn an_unanswered_setup_after_a_raw_frame_is_the_clients_doing() {
    let (deployment, compiled, execution) = run(&program(1, true));
    let mut unanswered = execution.clone();
    unanswered.outcomes[awaits(&compiled)[0]] = ActionOutcome::TimedOut;
    assert!(fails(SetupConnectionOracle, &deployment, &compiled, &unanswered).is_none());
}

/// Deleting any expected record makes an oracle fail, rather than pass over what it cannot see.
#[test]
fn a_missing_outcome_fails_every_oracle_that_expected_it() {
    let (deployment, compiled, execution) = run(&program(1, false));
    assert!(fails(HarnessIntegrityOracle, &deployment, &compiled, &execution).is_none());
    assert!(fails(SetupConnectionOracle, &deployment, &compiled, &execution).is_none());

    for missing in 0..execution.outcomes.len() {
        let mut truncated = execution.clone();
        truncated.outcomes.truncate(missing);
        let reason = fails(HarnessIntegrityOracle, &deployment, &compiled, &truncated)
            .unwrap_or_else(|| {
                panic!(
                    "integrity passed with {missing} of {} outcomes",
                    execution.outcomes.len()
                )
            });
        assert!(reason.contains("outcomes"), "{reason}");
    }

    let mut truncated = execution.clone();
    truncated.outcomes.truncate(awaits(&compiled)[0]);
    let reason = fails(SetupConnectionOracle, &deployment, &compiled, &truncated).unwrap();
    assert!(reason.contains("has no outcome"), "{reason}");
}

/// A harness error is the harness's failure, reported by the integrity oracle and by nothing
/// else.
#[test]
fn a_harness_error_is_an_integrity_failure_and_not_a_finding() {
    let (deployment, compiled, mut execution) = run(&program(1, false));
    execution.outcomes[awaits(&compiled)[0]] =
        ActionOutcome::HarnessError("could not encode".to_string());

    let reason = fails(HarnessIntegrityOracle, &deployment, &compiled, &execution).unwrap();
    assert!(reason.contains("could not encode"), "{reason}");
    assert!(fails(SetupConnectionOracle, &deployment, &compiled, &execution).is_none());
}
