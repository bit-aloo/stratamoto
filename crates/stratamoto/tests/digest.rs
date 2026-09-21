//! The digest tells a new behaviour from a repeat and nothing else.

use std::collections::HashSet;

use stratamoto::{
    deployment::SimulatedDeployment,
    digest::{ExecutionDigest, Verdict},
    events::Event,
    roles::RoleConfig,
    runner::{self, Execution},
    stratum_core::common_messages_sv2::Protocol as WireProtocol,
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::{CompiledProgram, Compiler},
};

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

/// `setups` setups for `protocol`, each on its own connection to role 0.
fn program(protocol: Protocol, setups: usize) -> Program {
    let mut b = ProgramBuilder::new(ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    });
    let role = one(b.append_op(Operation::LoadRole(0), &[]).unwrap());
    for _ in 0..setups {
        let connection = one(b.append_op(Operation::Connect, &[&role]).unwrap());
        let setup = one(b
            .append_op(Operation::BeginBuildSetupConnection, &[])
            .unwrap());
        let setup = one(b
            .append_op(Operation::EndBuildSetupConnection { protocol }, &[&setup])
            .unwrap());
        b.append_op(
            Operation::SendSetupConnection { protocol },
            &[&connection, &setup],
        )
        .unwrap();
    }
    b.finalize().unwrap()
}

fn run(seed: u64, protocol: Protocol, setups: usize) -> (CompiledProgram, Execution) {
    let deployment =
        SimulatedDeployment::new(seed, vec![RoleConfig::new(WireProtocol::MiningProtocol)]);
    let compiled = Compiler::new().compile(&program(protocol, setups)).unwrap();
    let execution = runner::run(&deployment, &compiled);
    (compiled, execution)
}

fn digest(compiled: &CompiledProgram, execution: &Execution) -> ExecutionDigest {
    ExecutionDigest::of(compiled, execution, true, Verdict::Ok)
}

/// The same program on the same deployment digests the same, run after run.
#[test]
fn a_repeat_digests_alike() {
    let (compiled, first) = run(1, Protocol::Mining, 1);
    let (_, second) = run(1, Protocol::Mining, 1);
    assert_eq!(digest(&compiled, &first), digest(&compiled, &second));
    assert_eq!(
        digest(&compiled, &first).hash(),
        digest(&compiled, &second).hash()
    );
}

/// A setup the role rejects is a different behaviour from one it agrees to.
#[test]
fn an_accepted_and_a_rejected_setup_differ() {
    let (accepted, execution) = run(1, Protocol::Mining, 1);
    let (rejected, execution_rejected) = run(1, Protocol::JobDeclaration, 1);
    let a = digest(&accepted, &execution);
    let b = digest(&rejected, &execution_rejected);
    assert_ne!(a, b);
    assert_eq!(a.counts.established, 1);
    assert_eq!(b.counts.established, 0);
}

/// The corpus admits on facts, not on whole digests: a run that shows only facts an earlier
/// run showed adds nothing, and one that shows a new fact adds that fact.
#[test]
fn features_are_the_facts_a_run_showed() {
    let (accepted, execution) = run(1, Protocol::Mining, 1);
    let seen: HashSet<u64> = digest(&accepted, &execution)
        .features()
        .into_iter()
        .collect();

    let (_, again) = run(2, Protocol::Mining, 1);
    let repeat: HashSet<u64> = digest(&accepted, &again).features().into_iter().collect();
    assert!(repeat.is_subset(&seen), "a repeat showed a new fact");

    let (rejected, execution_rejected) = run(1, Protocol::JobDeclaration, 1);
    let novel: HashSet<u64> = digest(&rejected, &execution_rejected)
        .features()
        .into_iter()
        .collect();
    assert!(!novel.is_subset(&seen), "a rejection showed nothing new");
}

/// Doing the same thing once more is not a new behaviour: an event seen twice in a row is one
/// transition, and a fourth connection is in the same bucket as a third.
#[test]
fn one_more_of_the_same_is_not_new() {
    let (compiled, execution) = run(1, Protocol::Mining, 3);
    let original = digest(&compiled, &execution);

    let mut repeated = execution.clone();
    let state = repeated.connections.get_mut(&0).unwrap();
    let last = state.events.last().unwrap().clone();
    state.events.push(last);
    assert_eq!(digest(&compiled, &repeated), original);

    let (four, execution_four) = run(1, Protocol::Mining, 4);
    assert_eq!(digest(&four, &execution_four).counts, original.counts);
}

/// What arrived unprompted is kept by class, not as one boolean, and it changes the digest.
#[test]
fn unsolicited_traffic_is_kept_by_class() {
    let (compiled, execution) = run(1, Protocol::Mining, 1);
    let original = digest(&compiled, &execution);

    let mut with_reconnect = execution.clone();
    with_reconnect.unsolicited.push((0, 0x04));
    with_reconnect
        .connections
        .get_mut(&0)
        .unwrap()
        .events
        .push(Event::Reconnect {
            host: "elsewhere".to_string(),
            port: 1,
        });
    let mut with_other = execution.clone();
    with_other.unsolicited.push((0, 0x21));
    with_other
        .connections
        .get_mut(&0)
        .unwrap()
        .events
        .push(Event::Other { message_type: 0x21 });

    let a = digest(&compiled, &with_reconnect);
    let b = digest(&compiled, &with_other);
    assert_ne!(a, original);
    assert_ne!(b, original);
    assert_ne!(a, b, "two kinds of unsolicited traffic are two behaviours");
    assert_eq!(a.unsolicited.len(), 1);
}

/// A verdict and the deployment's liveness are part of what a run did.
#[test]
fn the_verdict_is_part_of_the_digest() {
    let (compiled, execution) = run(1, Protocol::Mining, 1);
    let ok = ExecutionDigest::of(&compiled, &execution, true, Verdict::Ok);
    let failed = ExecutionDigest::of(
        &compiled,
        &execution,
        true,
        Verdict::Fail("CrashOracle".to_string()),
    );
    let dead = ExecutionDigest::of(&compiled, &execution, false, Verdict::Ok);
    assert_ne!(ok, failed);
    assert_ne!(ok, dead);
}
