use rand::{SeedableRng, rngs::SmallRng};
use stratamoto_fuzz::{
    Fuzzer,
    target::{Outcome, Target},
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
        Outcome::Ok { signature }
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
    fuzzer.seed(context());

    let failures = fuzzer.run(500);
    assert!(!failures.is_empty(), "never reached the planted failure");

    let failure = &failures[0];
    assert_eq!(failure.reason, "job declaration is not accepted");

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
            Outcome::Ok { signature: 0 }
        }
        fn name(&self) -> &'static str {
            "always"
        }
    }

    let mut fuzzer = Fuzzer::new(Always, SmallRng::seed_from_u64(2), 3);
    fuzzer.seed(context());
    assert!(fuzzer.run(200).is_empty());
    assert_eq!(fuzzer.stats().failures, 0);
}
