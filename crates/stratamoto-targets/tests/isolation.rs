//! Every run starts from a pool that has served nothing, and what that costs.
//!
//! The pool remembers what earlier connections did, so a run against a reused pool can see
//! what the runs before it left behind. Replacing the pool between runs is what makes a run's
//! result a property of the program alone.

use std::time::{Duration, Instant};

use stratamoto::{
    backend::ExecutionBackend,
    digest::Verdict,
    runner::{self, Execution, SetupResponse},
    transport::Deployment,
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::Compiler,
};
use stratamoto_targets::{
    backend::{Reset, RestartBackend},
    pool::PoolDeployment,
};

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

/// A mining setup, which the pool accepts.
fn accepted_setup() -> Program {
    setup(Protocol::Mining)
}

/// A setup the pool rejects: it does not serve template distribution.
fn rejected_setup() -> Program {
    setup(Protocol::TemplateDistribution)
}

fn setup(protocol: Protocol) -> Program {
    let mut b = ProgramBuilder::new(context());
    let role = one(b.append_op(Operation::LoadRole(0), &[]).unwrap());
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
    b.finalize().unwrap()
}

/// A rejected setup followed by a second frame in the pool's teardown window, which is the
/// trigger of the livelock the pool tests record. It wedges the pool about half the time,
/// which is what makes it the right thing to run before a case that must not care.
fn teardown_race() -> Program {
    let mut b = ProgramBuilder::new(context());
    let role = one(b.append_op(Operation::LoadRole(0), &[]).unwrap());
    let connection = one(b.append_op(Operation::Connect, &[&role]).unwrap());
    for protocol in [Protocol::TemplateDistribution, Protocol::Mining] {
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

fn run(deployment: &PoolDeployment, program: &Program) -> Execution {
    let compiled = Compiler::new().compile(program).unwrap();
    runner::run(deployment, &compiled)
}

/// What a run did, without the text of the pool's errors, which say which pool answered and
/// not whether the answer was the same kind.
#[derive(Debug, PartialEq, Eq)]
struct Normalized {
    sessions: Vec<(u8, bool, &'static str)>,
    alive: bool,
}

fn normalized(deployment: &PoolDeployment, execution: &Execution) -> Normalized {
    let mut sessions: Vec<_> = execution
        .sessions
        .values()
        .map(|s| {
            let kind = match s.response {
                SetupResponse::Success { .. } => "success",
                SetupResponse::Error { .. } => "error",
                SetupResponse::Unexpected { .. } => "unexpected",
                SetupResponse::Silence => "silence",
            };
            (s.protocol.id(), s.first_on_connection, kind)
        })
        .collect();
    sessions.sort_unstable();
    Normalized {
        sessions,
        alive: deployment.is_alive(),
    }
}

/// The exit criterion of the roadmap's first milestone: input B has the same normalized
/// result whether it runs alone, after input A, or after an input that left the pool broken.
#[test]
fn a_run_does_not_depend_on_what_ran_before_it() {
    let mut deployment = PoolDeployment::start().expect("the pool starts against a real node");
    let b = accepted_setup();

    let alone = run(&deployment, &b);
    let alone_normalized = normalized(&deployment, &alone);
    assert_eq!(alone_normalized.sessions, vec![(0, true, "success")]);

    // After a rejected setup, on a fresh pool.
    deployment.restart_pool().expect("the pool restarts");
    let _ = run(&deployment, &rejected_setup());
    deployment
        .restart_pool()
        .expect("the pool restarts after serving");
    let after_a = run(&deployment, &b);
    assert_eq!(normalized(&deployment, &after_a), alone_normalized);

    // After the run that may have wedged the pool, on a fresh pool. Whether it did is up to
    // the race; either way the replacement has to serve.
    deployment.restart_pool().expect("the pool restarts");
    let _ = run(&deployment, &teardown_race());
    let wedged = !deployment.is_alive();
    deployment
        .restart_pool()
        .expect("a pool that stopped serving is replaced like any other");
    assert!(deployment.is_alive(), "the replacement serves");
    let after_failure = run(&deployment, &b);
    assert_eq!(
        normalized(&deployment, &after_failure),
        alone_normalized,
        "after a run that {} the pool",
        if wedged { "wedged" } else { "spared" }
    );

    // On a reused pool the same setup is answered alike too. The invariant is enforced by
    // replacing the pool rather than by observing that, since what a deeper program observes
    // is exactly what is not known in advance.
    let reused = run(&deployment, &b);
    assert_eq!(normalized(&deployment, &reused), alone_normalized);
}

/// What the roadmap asks to record before choosing a restart policy: the cost of replacing the
/// pool alone, against a node and sv2-tp that keep running, and of replacing all three.
///
/// Run with `--nocapture` to see the numbers.
#[test]
fn a_clean_start_costs_this_much() {
    let mut deployment = PoolDeployment::start().expect("the pool starts against a real node");
    let program = accepted_setup();

    let time = |deployment: &mut PoolDeployment, restart: fn(&mut PoolDeployment)| {
        let started = Instant::now();
        restart(deployment);
        let elapsed = started.elapsed();
        // A restart only counts once the replacement answers a program.
        let execution = run(deployment, &program);
        assert_eq!(
            normalized(deployment, &execution).sessions,
            vec![(0, true, "success")]
        );
        elapsed
    };

    let pool_only: Vec<Duration> = (0..5)
        .map(|_| {
            time(&mut deployment, |d| {
                d.restart_pool().expect("the pool restarts")
            })
        })
        .collect();
    let everything: Vec<Duration> = (0..3)
        .map(|_| {
            time(&mut deployment, |d| {
                d.restart_all()
                    .expect("the node, sv2-tp and the pool restart")
            })
        })
        .collect();

    let median = |mut samples: Vec<Duration>| {
        samples.sort_unstable();
        samples[samples.len() / 2]
    };
    println!(
        "restart of the pool alone:       {pool_only:?}, median {:?}",
        median(pool_only.clone())
    );
    println!(
        "restart of the node, sv2-tp and the pool: {everything:?}, median {:?}",
        median(everything.clone())
    );
}

/// The backend hands every input a pool at the root, and its canary agrees, input after input.
///
/// Printed with `--nocapture`: what an input costs with the canary in front of it.
#[test]
fn the_backend_returns_to_the_root_between_inputs() {
    let mut backend = RestartBackend::new(Reset::Pool);
    backend.prepare_scenario().expect("the pool starts");
    let root = backend.take_root_snapshot().expect("the root is taken");
    assert_eq!(root.backend, "restart");

    let program = accepted_setup();
    let compiled = Compiler::new().compile(&program).unwrap();
    let mut costs = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let deployment = backend
            .next_input()
            .expect("the canary found the pool at the root");
        costs.push(started.elapsed());
        let execution = runner::run(deployment, &compiled);
        assert_eq!(
            normalized(deployment, &execution).sessions,
            vec![(0, true, "success")]
        );
        backend.report_and_reset(&Verdict::Ok).expect("reported");
    }
    println!("next_input with reset and canary: {costs:?}");
}

/// With resets turned off, a pool that stopped is not the root, and the canary says so before
/// the next input is blamed for what it finds.
#[test]
fn the_canary_trips_when_the_pool_is_not_at_the_root() {
    let mut backend = RestartBackend::new(Reset::None);
    backend.prepare_scenario().expect("the pool starts");
    backend.take_root_snapshot().expect("the root is taken");

    let compiled = Compiler::new().compile(&accepted_setup()).unwrap();
    let deployment = backend
        .next_input()
        .expect("the first input is at the root");
    let _ = runner::run(deployment, &compiled);
    backend.report_and_reset(&Verdict::Ok).expect("reported");

    backend.deployment_mut().expect("prepared").stop_pool();
    let error = match backend.next_input() {
        Ok(_) => panic!("the canary did not notice a pool that stopped"),
        Err(e) => e.to_string(),
    };
    assert!(error.contains("reset integrity"), "{error}");
}
