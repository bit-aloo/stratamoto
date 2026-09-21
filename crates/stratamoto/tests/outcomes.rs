//! Every action of a program records exactly one outcome, and the outcome says what happened.

use stratamoto::{
    deployment::SimulatedDeployment,
    roles::RoleConfig,
    runner::{self, ActionOutcome, Prerequisite, Response, SetupResponse},
    stratum_core::common_messages_sv2::Protocol as WireProtocol,
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::{Action, Compiler},
};

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

fn builder(num_roles: usize) -> ProgramBuilder {
    ProgramBuilder::new(ProgramContext {
        num_roles,
        num_connections: 0,
        seed: 0,
    })
}

/// A mining setup on `role`, then, once the server agreed, a frame of the program's own
/// choosing and a probe.
fn setup_then_more(role: usize, num_roles: usize) -> Program {
    let mut b = builder(num_roles);
    let role = one(b.append_op(Operation::LoadRole(role), &[]).unwrap());
    let connection = one(b.append_op(Operation::Connect, &[&role]).unwrap());
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
    let attempt = one(b
        .append_op(
            Operation::SendSetupConnection {
                protocol: Protocol::Mining,
            },
            &[&connection, &setup],
        )
        .unwrap());
    b.append_op(
        Operation::BeginOnSetupSuccess {
            protocol: Protocol::Mining,
        },
        &[&attempt],
    )
    .unwrap();
    let bytes = one(b.append_op(Operation::LoadBytes(vec![1, 2]), &[]).unwrap());
    b.append_op(
        Operation::SendRawFrame {
            message_type: 0x7f,
            extension_type: 0,
        },
        &[&connection, &bytes],
    )
    .unwrap();
    b.append_op(Operation::Probe, &[]).unwrap();
    b.append_op(Operation::EndOnSetupSuccess, &[]).unwrap();
    b.finalize().unwrap()
}

fn kinds(actions: &[Action]) -> Vec<&'static str> {
    actions
        .iter()
        .map(|a| match a {
            Action::Connect { .. } => "connect",
            Action::Send { .. } => "send",
            Action::AwaitSetupResponse { .. } => "await-setup",
            Action::EnterOnSetupSuccess { .. } => "enter",
            Action::ExitBlock => "exit",
            Action::AdvanceTime(_) => "advance",
            Action::Probe => "probe",
        })
        .collect()
}

/// Against a role that agrees to the setup, the block runs and every action in it has its
/// outcome; no action has two.
#[test]
fn every_action_has_exactly_one_outcome() {
    let deployment =
        SimulatedDeployment::new(1, vec![RoleConfig::new(WireProtocol::MiningProtocol)]);
    let compiled = Compiler::new().compile(&setup_then_more(0, 1)).unwrap();
    let execution = runner::run(&deployment, &compiled);

    assert!(execution.is_total(&compiled));
    assert_eq!(
        kinds(&compiled.actions),
        vec![
            "connect",
            "send",
            "await-setup",
            "enter",
            "send",
            "probe",
            "exit"
        ]
    );
    assert_eq!(
        execution.outcomes,
        vec![
            ActionOutcome::Completed(Response::None),
            ActionOutcome::Completed(Response::None),
            ActionOutcome::Completed(Response::Setup(SetupResponse::Success {
                used_version: 2,
                flags: 0
            })),
            ActionOutcome::Completed(Response::None),
            ActionOutcome::Completed(Response::None),
            ActionOutcome::Completed(Response::Probed(vec![])),
            ActionOutcome::Completed(Response::None),
        ]
    );
    assert_eq!(execution.sessions.len(), 1);
}

/// A connection that cannot be opened is a transport failure, and everything that needed it
/// is skipped rather than silently absent.
#[test]
fn what_needed_a_connection_that_never_opened_is_skipped() {
    // The program addresses a second role the deployment does not have.
    let deployment =
        SimulatedDeployment::new(1, vec![RoleConfig::new(WireProtocol::MiningProtocol)]);
    let compiled = Compiler::new().compile(&setup_then_more(1, 2)).unwrap();
    let execution = runner::run(&deployment, &compiled);

    assert!(execution.is_total(&compiled));
    assert!(matches!(
        execution.outcomes[0],
        ActionOutcome::TransportError(_)
    ));
    for outcome in &execution.outcomes[1..3] {
        assert_eq!(
            *outcome,
            ActionOutcome::Skipped(Prerequisite::ConnectionOpen(0))
        );
    }
    // With no setup answered, the block that waits for one is skipped whole.
    for outcome in &execution.outcomes[3..] {
        assert_eq!(
            *outcome,
            ActionOutcome::Skipped(Prerequisite::SetupSuccess(0))
        );
    }
    assert!(execution.sessions.is_empty(), "no session was recorded");
}

/// A setup the role rejects skips the block that needed it to succeed, so nothing that
/// presumes a session goes out on a connection that has none.
#[test]
fn a_rejected_setup_skips_what_needed_it() {
    // A template distribution role rejects a mining setup.
    let deployment = SimulatedDeployment::new(
        1,
        vec![RoleConfig::new(WireProtocol::TemplateDistributionProtocol)],
    );
    let compiled = Compiler::new().compile(&setup_then_more(0, 1)).unwrap();
    let execution = runner::run(&deployment, &compiled);

    assert!(execution.is_total(&compiled));
    assert!(matches!(
        execution.outcomes[2],
        ActionOutcome::Completed(Response::Setup(SetupResponse::Error { .. }))
    ));
    for outcome in &execution.outcomes[3..] {
        assert_eq!(
            *outcome,
            ActionOutcome::Skipped(Prerequisite::SetupSuccess(0))
        );
    }
    assert!(execution.unsolicited.is_empty(), "the probe never ran");
}

/// Actions that await nothing complete, and a probe reports what it collected.
#[test]
fn waits_and_probes_complete() {
    let deployment =
        SimulatedDeployment::new(1, vec![RoleConfig::new(WireProtocol::MiningProtocol)]);
    let mut b = builder(1);
    let duration = one(b
        .append_op(
            Operation::LoadDuration(std::time::Duration::from_millis(5)),
            &[],
        )
        .unwrap());
    b.append_op(Operation::AdvanceTime, &[&duration]).unwrap();
    b.append_op(Operation::Probe, &[]).unwrap();
    let compiled = Compiler::new().compile(&b.finalize().unwrap()).unwrap();
    let execution = runner::run(&deployment, &compiled);

    assert_eq!(
        execution.outcomes,
        vec![
            ActionOutcome::Completed(Response::None),
            ActionOutcome::Completed(Response::Probed(vec![])),
        ]
    );
}
