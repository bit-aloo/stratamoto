use std::{io::Write, process::ExitCode};

use rand::{SeedableRng, rngs::SmallRng};
use stratamoto::{
    runner::Execution,
    scenario::{Scenario, ScenarioResult},
};
use stratamoto_fuzz::{
    Fuzzer,
    target::{Outcome, Target},
};
use stratamoto_ir::{Program, ProgramContext};
use stratamoto_scenarios::{
    pool_setup_connection::PoolSetupConnectionScenario,
    setup_connection::{SetupConnectionScenario, TestCase, roles, signature},
};

const USAGE: &str = "\
usage: stratamoto-fuzz [iterations] [seed] [failure directory]

STRATAMOTO_TARGET selects what is fuzzed:
  simulated  the mock roles on the deterministic simulator (default)
  pool       sv2-apps' pool, against a Template Provider the harness plays
";

struct SimulatedTarget {
    scenario: SetupConnectionScenario,
}

impl Target for SimulatedTarget {
    fn run(&mut self, program: &Program) -> Outcome {
        outcome(program, |testcase| self.scenario.execute(testcase))
    }

    fn name(&self) -> &'static str {
        "setup_connection"
    }
}

struct PoolTarget {
    scenario: PoolSetupConnectionScenario,
}

impl Target for PoolTarget {
    fn run(&mut self, program: &Program) -> Outcome {
        outcome(program, |testcase| self.scenario.execute(testcase))
    }

    fn name(&self) -> &'static str {
        "pool_setup_connection"
    }

    fn is_alive(&self) -> bool {
        self.scenario.is_alive()
    }
}

fn outcome(
    program: &Program,
    execute: impl FnOnce(&TestCase) -> (Execution, ScenarioResult),
) -> Outcome {
    let Ok(testcase) = TestCase::from_program(program) else {
        return Outcome::Skip;
    };
    let (execution, result) = execute(&testcase);
    match result {
        ScenarioResult::Ok => Outcome::Ok {
            signature: signature(&execution),
        },
        ScenarioResult::Skip => Outcome::Skip,
        ScenarioResult::Fail(reason) => Outcome::Fail(reason),
    }
}

fn main() -> ExitCode {
    env_logger::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let parse = |i: usize, default: u64| -> Option<u64> {
        args.get(i).map_or(Some(default), |s| s.parse().ok())
    };
    let (Some(iterations), Some(seed)) = (parse(0, 10_000), parse(1, 0)) else {
        eprint!("{USAGE}");
        return ExitCode::FAILURE;
    };
    let failure_dir = args.get(2).map(String::as_str);

    match std::env::var("STRATAMOTO_TARGET").as_deref() {
        Err(_) | Ok("simulated") => {
            let target = SimulatedTarget {
                scenario: SetupConnectionScenario::new(seed).expect("the scenario takes any seed"),
            };
            fuzz(target, roles().len(), iterations, seed, failure_dir)
        }
        Ok("pool") => match PoolSetupConnectionScenario::start() {
            Ok(scenario) => {
                let num_roles = scenario.num_roles();
                fuzz(
                    PoolTarget { scenario },
                    num_roles,
                    iterations,
                    seed,
                    failure_dir,
                )
            }
            Err(e) => {
                eprintln!("could not start the pool: {e}");
                ExitCode::FAILURE
            }
        },
        Ok(other) => {
            eprint!("unknown target {other:?}\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn fuzz<T: Target>(
    target: T,
    num_roles: usize,
    iterations: u64,
    seed: u64,
    failure_dir: Option<&str>,
) -> ExitCode {
    let context = ProgramContext {
        num_roles,
        num_connections: 0,
        seed,
    };

    let mut fuzzer = Fuzzer::new(target, SmallRng::seed_from_u64(seed), num_roles);
    fuzzer.seed(context);

    let failures = fuzzer.run(iterations);

    let stats = fuzzer.stats();
    println!(
        "{} iterations, {} corpus, {} rejected, {} skipped, {} failures",
        stats.iterations,
        fuzzer.corpus().len(),
        stats.rejected,
        stats.skipped,
        stats.failures
    );

    for (i, failure) in failures.iter().enumerate() {
        let note = if failure.killed_the_target {
            " (target stopped serving; reported unminimized)"
        } else {
            ""
        };
        println!("\n--- failure {i}{note}: {} ---", failure.reason);
        print!("{}", failure.program);

        if let Some(dir) = failure_dir
            && let Err(e) = save(dir, i, &failure.program)
        {
            eprintln!("could not save failure {i}: {e}");
        }
    }

    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn save(dir: &str, index: usize, program: &Program) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = std::path::Path::new(dir).join(format!("failure-{index}.postcard"));
    let bytes = postcard::to_allocvec(program).map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::File::create(&path)?.write_all(&bytes)?;
    println!("saved to {}", path.display());
    Ok(())
}
