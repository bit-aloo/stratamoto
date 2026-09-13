use std::io::Write;

use rand::{SeedableRng, rngs::SmallRng};
use stratamoto::scenario::{Scenario, ScenarioResult};
use stratamoto_fuzz::{
    Fuzzer,
    target::{Outcome, Target},
};
use stratamoto_ir::{Program, ProgramContext};
use stratamoto_scenarios::setup_connection::{
    SetupConnectionScenario, TestCase, roles, signature,
};

const USAGE: &str = "\
usage: stratamoto-fuzz [iterations] [seed] [failure directory]
";

/// The setup connection scenario, driven by the fuzzer.
struct SetupConnectionTarget {
    scenario: SetupConnectionScenario,
}

impl Target for SetupConnectionTarget {
    fn run(&mut self, program: &Program) -> Outcome {
        let Ok(testcase) = TestCase::from_program(program) else {
            return Outcome::Skip;
        };
        let (execution, result) = self.scenario.execute(&testcase);
        match result {
            ScenarioResult::Ok => Outcome::Ok {
                signature: signature(&execution),
            },
            ScenarioResult::Skip => Outcome::Skip,
            ScenarioResult::Fail(reason) => Outcome::Fail(reason),
        }
    }

    fn name(&self) -> &'static str {
        "setup_connection"
    }
}

fn main() -> std::process::ExitCode {
    env_logger::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let parse = |i: usize, default: u64| -> Option<u64> {
        args.get(i).map_or(Some(default), |s| s.parse().ok())
    };
    let (Some(iterations), Some(seed)) = (parse(0, 10_000), parse(1, 0)) else {
        eprint!("{USAGE}");
        return std::process::ExitCode::FAILURE;
    };
    let failure_dir = args.get(2).cloned();

    let num_roles = roles().len();
    let context = ProgramContext {
        num_roles,
        num_connections: 0,
        seed,
    };

    let target = SetupConnectionTarget {
        scenario: SetupConnectionScenario::new(seed).expect("the scenario takes any seed"),
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
        println!("\n--- failure {i}: {} ---", failure.reason);
        print!("{}", failure.program);

        if let Some(dir) = &failure_dir {
            if let Err(e) = save(dir, i, &failure.program) {
                eprintln!("could not save failure {i}: {e}");
            }
        }
    }

    if failures.is_empty() {
        std::process::ExitCode::SUCCESS
    } else {
        std::process::ExitCode::FAILURE
    }
}

fn save(dir: &str, index: usize, program: &Program) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = std::path::Path::new(dir).join(format!("failure-{index}.postcard"));
    let bytes = postcard::to_allocvec(program)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::File::create(&path)?.write_all(&bytes)?;
    println!("saved to {}", path.display());
    Ok(())
}
