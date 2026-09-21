use std::{io::Write, process::ExitCode};

use rand::{SeedableRng, rngs::SmallRng};
use stratamoto::scenario::ScenarioResult;
use stratamoto_fuzz::{
    Fuzzer,
    target::{Behaviour, Outcome, Target},
};
use stratamoto_ir::{
    Program, ProgramContext,
    artifact::{Artifact, Campaign, Revision},
};
use stratamoto_scenarios::{
    pool_setup_connection::{PoolSetupConnectionScenario, Reset},
    setup_connection::{Run, SetupConnectionScenario, TestCase, roles},
};

const USAGE: &str = "\
usage: stratamoto-fuzz [iterations] [seed] [failure directory]

STRATAMOTO_TARGET selects what is fuzzed:
  simulated  the mock roles on the deterministic simulator (default)
  pool       sv2-apps' pool, against a Template Provider the harness plays

STRATAMOTO_RESET selects what the pool target replaces before each run:
  pool       the pool alone; the node and sv2-tp keep running (default)
  all        the pool, sv2-tp and the node
  none       nothing, so a run sees what earlier runs left in the pool

STRATAMOTO_CORPUS names a directory the corpus is kept in: what an earlier campaign saved
there is taken back at the start, and every entry admitted is saved there as it is.

The seed drives the campaign and, for the simulated roles, the deployment each program runs on.
Failures written to the directory are artifacts a scenario binary replays as they are.
";

struct SimulatedTarget {
    scenario: SetupConnectionScenario,
    last: Option<Vec<u8>>,
}

impl Target for SimulatedTarget {
    fn run(&mut self, program: &Program) -> Outcome {
        let (outcome, trace) = outcome(program, |testcase| self.scenario.execute(testcase));
        self.last = trace;
        outcome
    }

    fn name(&self) -> &'static str {
        "setup_connection"
    }

    fn trace(&self) -> Option<Vec<u8>> {
        self.last.clone()
    }
}

struct PoolTarget {
    scenario: PoolSetupConnectionScenario,
    last: Option<Vec<u8>>,
}

impl Target for PoolTarget {
    fn run(&mut self, program: &Program) -> Outcome {
        let (outcome, trace) = outcome(program, |testcase| self.scenario.execute(testcase));
        self.last = trace;
        outcome
    }

    fn name(&self) -> &'static str {
        "pool_setup_connection"
    }

    fn is_alive(&self) -> bool {
        self.scenario.is_alive()
    }

    fn trace(&self) -> Option<Vec<u8>> {
        self.last.clone()
    }
}

/// Run a program and say what came of it, with the run's record and digest encoded for an
/// artifact. The corpus is keyed by the digest, so a program that drew the same kinds of
/// answers and reached the same states as another is a repeat, whatever numbers the server
/// chose.
fn outcome(
    program: &Program,
    execute: impl FnOnce(&TestCase) -> Run,
) -> (Outcome, Option<Vec<u8>>) {
    let Ok(testcase) = TestCase::from_program(program) else {
        return (Outcome::Skip, None);
    };
    let run = execute(&testcase);
    log::debug!("digest: {:?}", run.digest);
    let trace = postcard::to_allocvec(&(&run.execution, &run.digest)).ok();
    let outcome = match run.result {
        ScenarioResult::Ok => Outcome::Ok(Behaviour {
            features: run.digest.features(),
        }),
        ScenarioResult::Skip => Outcome::Skip,
        ScenarioResult::Fail(reason) => Outcome::Fail(reason),
        ScenarioResult::Infrastructure(reason) => Outcome::Infrastructure(reason),
    };
    (outcome, trace)
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
                scenario: SetupConnectionScenario,
                last: None,
            };
            fuzz(target, roles().len(), iterations, seed, failure_dir)
        }
        Ok("pool") => {
            let reset = match std::env::var("STRATAMOTO_RESET") {
                Err(_) => Reset::Pool,
                Ok(name) => match Reset::parse(&name) {
                    Some(reset) => reset,
                    None => {
                        eprint!("unknown reset policy {name:?}\n{USAGE}");
                        return ExitCode::FAILURE;
                    }
                },
            };
            match PoolSetupConnectionScenario::start_with(reset) {
                Ok(scenario) => {
                    if let Some(snapshot) = scenario.snapshot() {
                        println!("root snapshot: {snapshot}");
                    }
                    let num_roles = scenario.num_roles();
                    fuzz(
                        PoolTarget {
                            scenario,
                            last: None,
                        },
                        num_roles,
                        iterations,
                        seed,
                        failure_dir,
                    )
                }
                Err(e) => {
                    eprintln!("infrastructure failure: could not start the pool: {e}");
                    ExitCode::from(2)
                }
            }
        }
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

    let scenario = target.name();
    let mut fuzzer = Fuzzer::new(target, SmallRng::seed_from_u64(seed), num_roles);
    #[cfg(stratamoto_coverage)]
    {
        fuzzer = fuzzer.with_observer(Box::new(stratamoto_fuzz::observer::LlvmCoverage::new()));
    }
    if let Ok(dir) = std::env::var("STRATAMOTO_CORPUS") {
        fuzzer = match fuzzer.with_corpus_dir(&dir) {
            Ok(fuzzer) => fuzzer,
            Err(e) => {
                eprintln!("infrastructure failure: could not use the corpus directory {dir}: {e}");
                return ExitCode::from(2);
            }
        };
        match fuzzer.load_corpus(std::path::Path::new(&dir)) {
            Ok(loaded) => println!("{loaded} corpus entries taken back from {dir}"),
            Err(e) => {
                eprintln!("infrastructure failure: could not read the corpus directory {dir}: {e}");
                return ExitCode::from(2);
            }
        }
    }
    let mut failures = fuzzer.seed(context);
    let seeded = failures.len();

    if failures.iter().any(|failure| failure.killed_the_target) {
        eprintln!("the target stopped serving while seeding; skipping the campaign");
    } else if fuzzer.stopped().is_none() {
        failures.extend(fuzzer.run(iterations));
    }

    let stats = fuzzer.stats();
    println!(
        "{} iterations, {} corpus, {} rejected, {} skipped, {} failures",
        stats.iterations,
        fuzzer.corpus().len(),
        stats.rejected,
        stats.skipped,
        stats.failures
    );
    if cfg!(stratamoto_coverage) {
        println!(
            "{} regions reached ({}); {} corpus entries kept for coverage alone",
            fuzzer.coverage(),
            fuzzer.observer_name(),
            stats.coverage_additions
        );
    } else {
        println!(
            "no coverage from inside the target: build with RUSTFLAGS=\"-C instrument-coverage\""
        );
    }

    for (i, failure) in failures.iter().enumerate() {
        let origin = if i < seeded {
            " (found while seeding)"
        } else {
            ""
        };
        let note = if failure.killed_the_target {
            " (target stopped serving; reported unminimized)"
        } else if failure.confirmed {
            " (confirmed)"
        } else {
            " (did not reproduce when run again)"
        };
        println!("\n--- failure {i}{origin}{note}: {} ---", failure.reason);
        print!("{}", failure.program);

        if let Some(dir) = failure_dir {
            let artifact = Artifact {
                program: failure.program.clone(),
                scenario: scenario.to_string(),
                revisions: revisions(),
                campaign: Some(Campaign {
                    seed,
                    iteration: failure.iteration,
                    parent: failure.parent.clone(),
                }),
                verdict: failure.reason.clone(),
                confirmed: failure.confirmed,
                trace: failure.trace.clone(),
            };
            if let Err(e) = save(dir, i, &artifact) {
                eprintln!("could not save failure {i}: {e}");
            }
        }
    }

    // A finding outranks an outage in the exit code, since it is the one that will not
    // reproduce itself; the outage is on stderr either way.
    if let Some(reason) = fuzzer.stopped() {
        eprintln!("\ninfrastructure failure ended the campaign: {reason}");
    }
    if !failures.is_empty() {
        ExitCode::FAILURE
    } else if fuzzer.stopped().is_some() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn save(dir: &str, index: usize, artifact: &Artifact) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = std::path::Path::new(dir).join(format!("failure-{index}.stratamoto"));
    let bytes = artifact.encode().map_err(std::io::Error::other)?;
    std::fs::File::create(&path)?.write_all(&bytes)?;
    println!("saved to {}", path.display());
    Ok(())
}

/// What this binary was built against, as the build script read it from the lock file and the
/// checkout.
fn revisions() -> Vec<Revision> {
    env!("STRATAMOTO_REVISIONS")
        .split(';')
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let mut fields = entry.splitn(3, ' ');
            let field = |f: &mut std::str::SplitN<'_, char>| f.next().unwrap_or("").to_string();
            Revision {
                name: field(&mut fields),
                version: field(&mut fields),
                source: field(&mut fields),
            }
        })
        .collect()
}
