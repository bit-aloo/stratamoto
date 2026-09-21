use rand::{SeedableRng, rngs::SmallRng};
use stratamoto_fuzz::{
    Fuzzer,
    observer::Observer,
    target::{Behaviour, Outcome, Target},
};
use stratamoto_ir::{Operation, Program, ProgramContext, Protocol};

/// A target that refuses job declaration setups, standing in for a role with a bug the
/// fuzzer has to reach by mutating the protocol of a generated program.
#[derive(Default)]
struct PickyTarget {
    runs: u64,
}

impl Target for PickyTarget {
    fn run(&mut self, program: &Program) -> Outcome {
        self.runs += 1;

        let offends = program.instructions.iter().any(|instruction| {
            matches!(
                instruction.operation,
                Operation::SendSetupConnection {
                    protocol: Protocol::JobDeclaration
                }
            )
        });

        if offends {
            return Outcome::Fail("job declaration is not accepted".to_string());
        }

        // Distinguish runs by which protocols they set up, so the corpus stays meaningful.
        let signature = program
            .instructions
            .iter()
            .filter_map(|instruction| match instruction.operation {
                Operation::SendSetupConnection { protocol } => Some(protocol.id() as u64 + 1),
                _ => None,
            })
            .sum();
        Outcome::Ok(Behaviour::single(signature))
    }

    fn name(&self) -> &'static str {
        "picky"
    }
}

fn context() -> ProgramContext {
    ProgramContext {
        num_roles: 3,
        num_connections: 0,
        seed: 0,
    }
}

#[test]
fn the_fuzzer_finds_and_minimizes_a_planted_failure() {
    let mut fuzzer = Fuzzer::new(PickyTarget::default(), SmallRng::seed_from_u64(1), 3);

    // A generated seed may already pick the protocol the target refuses, in which case the
    // planted failure is found before the campaign starts.
    let mut failures = fuzzer.seed(context());
    failures.extend(fuzzer.run(500));
    assert!(!failures.is_empty(), "never reached the planted failure");

    let failure = &failures[0];
    assert_eq!(failure.reason, "job declaration is not accepted");

    // Where it came from is part of the finding: a seed program was generated at iteration
    // zero, anything else was mutated from a corpus entry at some later one.
    if failure.parent.is_some() {
        assert!(failure.iteration >= 1, "a mutant found before the campaign");
    } else {
        assert_eq!(failure.iteration, 0, "a seed found during the campaign");
    }

    // The reproducer has to still fail, and carry nothing that is not needed to: a role, a
    // connection, the block that builds the message, and the send.
    let program = &failure.program;
    assert!(program.is_statically_valid(), "reproducer is invalid");
    assert!(
        program.instructions.iter().any(|i| matches!(
            i.operation,
            Operation::SendSetupConnection {
                protocol: Protocol::JobDeclaration
            }
        )),
        "reproducer no longer triggers the failure:\n{program}"
    );
    assert_eq!(
        program.instructions.len(),
        5,
        "reproducer was not fully minimized:\n{program}"
    );
}

#[test]
fn the_fuzzer_leaves_a_correct_target_alone() {
    struct Always;
    impl Target for Always {
        fn run(&mut self, _program: &Program) -> Outcome {
            Outcome::Ok(Behaviour::single(0))
        }
        fn name(&self) -> &'static str {
            "always"
        }
    }

    let mut fuzzer = Fuzzer::new(Always, SmallRng::seed_from_u64(2), 3);
    assert!(fuzzer.seed(context()).is_empty());
    assert!(fuzzer.run(200).is_empty());
    assert_eq!(fuzzer.stats().failures, 0);
}

/// A target that fails every program fails the seeds too, and those failures are reported
/// rather than dropped on the way to an empty corpus.
#[test]
fn the_fuzzer_reports_failures_found_while_seeding() {
    struct Never;
    impl Target for Never {
        fn run(&mut self, _program: &Program) -> Outcome {
            Outcome::Fail("nothing is accepted".to_string())
        }
        fn name(&self) -> &'static str {
            "never"
        }
    }

    let mut fuzzer = Fuzzer::new(Never, SmallRng::seed_from_u64(3), 3);
    let failures = fuzzer.seed(context());

    assert_eq!(failures.len(), 3, "one seed per role, each failing");
    assert_eq!(fuzzer.stats().failures, 3);
    assert_eq!(fuzzer.corpus().len(), 0);
    for failure in &failures {
        assert_eq!(failure.reason, "nothing is accepted");
        assert!(!failure.killed_the_target);
        assert_eq!(failure.iteration, 0);
        assert!(failure.parent.is_none());
        // Every candidate fails the same way, so the reproducer minimizes to nothing.
        assert!(
            failure.program.instructions.is_empty(),
            "{}",
            failure.program
        );
    }
}

/// A target the harness can no longer run against stops the campaign with a verdict of its
/// own, rather than counting as a finding or as a dead target.
#[test]
fn an_infrastructure_failure_stops_the_campaign() {
    struct Outage {
        runs: u64,
    }
    impl Target for Outage {
        fn run(&mut self, _program: &Program) -> Outcome {
            self.runs += 1;
            if self.runs > 5 {
                Outcome::Infrastructure("the pool could not be reset".to_string())
            } else {
                Outcome::Ok(Behaviour::single(self.runs))
            }
        }
        fn name(&self) -> &'static str {
            "outage"
        }
    }

    let mut fuzzer = Fuzzer::new(Outage { runs: 0 }, SmallRng::seed_from_u64(4), 3);
    assert!(fuzzer.seed(context()).is_empty());
    assert!(fuzzer.stopped().is_none());

    let failures = fuzzer.run(200);
    assert!(failures.is_empty(), "an outage is not a finding");
    assert_eq!(fuzzer.stopped(), Some("the pool could not be reset"));
    assert_eq!(fuzzer.stats().infrastructure, 1);
    assert_eq!(fuzzer.stats().failures, 0);
    assert!(fuzzer.stats().iterations < 200, "the campaign did not stop");

    // And it stays stopped.
    assert!(fuzzer.run(10).is_empty());
    assert_eq!(fuzzer.stats().infrastructure, 1);
}

/// A target every run of which behaves alike fills one corpus slot on its own; an observer
/// that sees something new inside it keeps more.
#[test]
fn an_observer_keeps_an_input_the_signature_would_discard() {
    struct Same;
    impl Target for Same {
        fn run(&mut self, _program: &Program) -> Outcome {
            Outcome::Ok(Behaviour::single(0))
        }
        fn name(&self) -> &'static str {
            "same"
        }
    }

    /// Reports every third run as reaching something new.
    struct EveryThird {
        runs: usize,
    }
    impl Observer for EveryThird {
        fn reset(&mut self) {}
        fn observe(&mut self) -> bool {
            self.runs += 1;
            self.runs.is_multiple_of(3)
        }
        fn seen(&self) -> usize {
            self.runs / 3
        }
        fn name(&self) -> &'static str {
            "every-third"
        }
    }

    let mut without = Fuzzer::new(Same, SmallRng::seed_from_u64(5), 3);
    assert!(without.seed(context()).is_empty());
    assert!(without.run(60).is_empty());
    assert_eq!(without.corpus().len(), 1, "one behaviour, one slot");
    assert_eq!(without.stats().coverage_additions, 0);

    let mut with = Fuzzer::new(Same, SmallRng::seed_from_u64(5), 3)
        .with_observer(Box::new(EveryThird { runs: 0 }));
    assert!(with.seed(context()).is_empty());
    assert!(with.run(60).is_empty());
    let stats = with.stats();
    assert!(stats.coverage_additions >= 1, "{stats:?}");
    assert_eq!(
        stats.corpus_additions,
        1 + stats.coverage_additions,
        "{stats:?}"
    );
    assert_eq!(with.corpus().len() as u64, stats.corpus_additions);
    assert_eq!(with.coverage() as u64, stats.coverage_additions);
}

/// A target that tells programs apart by how many setups they send, so the corpus grows.
struct Counting;
impl Target for Counting {
    fn run(&mut self, program: &Program) -> Outcome {
        let setups = program
            .instructions
            .iter()
            .filter(|i| matches!(i.operation, Operation::SendSetupConnection { .. }))
            .count();
        Outcome::Ok(Behaviour::single(setups as u64))
    }
    fn name(&self) -> &'static str {
        "counting"
    }
}

/// What one campaign admits, a later one starts from.
#[test]
fn the_corpus_is_saved_and_taken_back() {
    let dir = std::env::temp_dir().join(format!(
        "stratamoto-corpus-{}-{}",
        std::process::id(),
        line!()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let mut first = Fuzzer::new(Counting, SmallRng::seed_from_u64(6), 3)
        .with_corpus_dir(&dir)
        .unwrap();
    assert!(first.seed(context()).is_empty());
    assert!(first.run(100).is_empty());
    let saved = first.corpus().len();
    assert!(saved >= 2, "the corpus did not grow: {saved}");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), saved);

    let mut second = Fuzzer::new(Counting, SmallRng::seed_from_u64(7), 3)
        .with_corpus_dir(&dir)
        .unwrap();
    assert_eq!(second.load_corpus(&dir).unwrap(), saved);
    assert_eq!(second.stats().loaded as usize, saved);
    assert_eq!(second.corpus().len(), saved);
    let mut before: Vec<Program> = first.corpus().programs().cloned().collect();
    let mut after: Vec<Program> = second.corpus().programs().cloned().collect();
    before.sort_by_key(|p| format!("{p}"));
    after.sort_by_key(|p| format!("{p}"));
    assert_eq!(before, after);

    // Seeds the earlier campaign already had are not admitted twice, and the files it wrote
    // are not touched by what this one admits.
    assert!(second.seed(context()).is_empty());
    assert_eq!(second.corpus().len(), saved);
    assert!(second.run(50).is_empty());
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        second.corpus().len()
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An entry whose mutants keep earning slots is picked more often than one that never did.
#[test]
fn promising_entries_are_picked_more_often() {
    use stratamoto_fuzz::Corpus;
    let mut corpus = Corpus::new();
    let mut rng = SmallRng::seed_from_u64(8);
    let builder = stratamoto_ir::ProgramBuilder::new(context());
    let empty = builder.finalize().unwrap();
    corpus.add(empty.clone(), Behaviour::single(1));
    corpus.add(empty, Behaviour::single(2));
    for _ in 0..10 {
        corpus.credit(1);
    }

    let mut picks = [0u32; 2];
    for _ in 0..1000 {
        picks[corpus.pick(&mut rng).unwrap()] += 1;
    }
    assert!(
        picks[1] > 2 * picks[0],
        "the credited entry was picked {} times, the other {}",
        picks[1],
        picks[0]
    );
}
