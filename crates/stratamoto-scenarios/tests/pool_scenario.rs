//! The scenario binary, run the way a fuzzer's share directory or a script runs it.
//!
//! Needs the pool binary `STRATAMOTO_POOL` names, and Bitcoin Core.

use std::{
    io::Write,
    process::{Command, Stdio},
};

use rand::SeedableRng;
use stratamoto_ir::{
    Operation, ProgramBuilder, ProgramContext,
    generators::{Generator, setup_connection::SetupConnectionGenerator},
};

const SCENARIO: &str = env!("CARGO_BIN_EXE_pool_setup_connection");

fn pool() -> String {
    std::env::var("STRATAMOTO_POOL").expect("STRATAMOTO_POOL names sv2-apps' pool binary")
}

/// Run the scenario binary on `input`, the pool given as its argument, and return the exit
/// code and what it logged.
fn run(input: &[u8]) -> (i32, String) {
    let mut child = Command::new(SCENARIO)
        .arg(pool())
        .env("RUST_LOG", "info")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the scenario binary runs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input)
        .expect("the input is written");
    let output = child.wait_with_output().expect("the scenario exits");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn program(num_roles: usize) -> Vec<u8> {
    let mut rng = rand::rngs::SmallRng::seed_from_u64(7);
    let mut builder = ProgramBuilder::new(ProgramContext {
        num_roles,
        num_connections: 0,
        seed: 0,
    });
    SetupConnectionGenerator::new(0)
        .generate(&mut builder, &mut rng)
        .unwrap();
    postcard::to_allocvec(&builder.finalize().unwrap()).unwrap()
}

#[test]
fn the_scenario_binary_runs_one_program_against_the_pool() {
    let (code, log) = run(&program(1));
    assert_eq!(code, 0, "{log}");
    assert!(log.contains("the test case passed"), "{log}");
}

/// Bytes that are not a program, and a program for more roles than the pool has, are both
/// skipped: they say nothing about the pool.
#[test]
fn inputs_that_are_not_a_test_case_for_this_pool_are_skipped() {
    let (code, log) = run(b"not a program");
    assert_eq!(code, 0, "{log}");
    assert!(log.contains("skipping"), "{log}");

    let (code, log) = run(&program(3));
    assert_eq!(code, 0, "{log}");
    assert!(log.contains("skipping"), "{log}");
}

/// The scenario tells the fuzzer what context to write programs in.
#[test]
fn the_scenario_dumps_its_program_context() {
    let dump = std::env::temp_dir().join(format!("stratamoto-context-{}", std::process::id()));
    let status = Command::new(SCENARIO)
        .arg(pool())
        .env("STRATAMOTO_DUMP_IR_CONTEXT", &dump)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("the scenario binary runs");
    assert!(status.success());

    let context: ProgramContext = postcard::from_bytes(&std::fs::read(&dump).unwrap()).unwrap();
    let _ = std::fs::remove_file(&dump);
    assert_eq!(context.num_roles, 1);

    // And that context is one a program can be written in.
    let mut builder = ProgramBuilder::new(context);
    let role = builder.append_op(Operation::LoadRole(0), &[]).unwrap();
    builder.append_op(Operation::Connect, &[&role[0]]).unwrap();
    assert!(builder.finalize().is_ok());
}
