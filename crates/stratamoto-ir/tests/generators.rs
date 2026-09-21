//! The valid generator does what a conforming client does; the adversarial one departs from
//! it in exactly one named way.

use rand::{SeedableRng, rngs::SmallRng};
use stratamoto_ir::{
    Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::{Action, ClientViolation, CompiledProgram, Compiler, SetupConnectionSpec},
    generators::{
        Generator,
        setup_connection::{
            AdversarialSetupGenerator, CURRENT_VERSION, ConnectionChoice, SetupConnectionGenerator,
            SetupViolation, defined_flags,
        },
    },
};

fn context() -> ProgramContext {
    ProgramContext {
        num_roles: 2,
        num_connections: 0,
        seed: 0,
    }
}

fn generate<G: Generator<SmallRng>>(generator: &G, seed: u64, times: usize) -> Program {
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut builder = ProgramBuilder::new(context());
    for _ in 0..times {
        generator.generate(&mut builder, &mut rng).unwrap();
    }
    builder.finalize().unwrap()
}

fn compile(program: &Program) -> CompiledProgram {
    Compiler::new().compile(program).unwrap()
}

fn setups(compiled: &CompiledProgram) -> Vec<SetupConnectionSpec> {
    let mut setups: Vec<_> = compiled.metadata.session_setups.iter().collect();
    setups.sort_by_key(|(id, _)| **id);
    setups.into_iter().map(|(_, s)| s.clone()).collect()
}

fn connects(program: &Program) -> usize {
    program
        .instructions
        .iter()
        .filter(|i| matches!(i.operation, Operation::Connect))
        .count()
}

fn violations(compiled: &CompiledProgram) -> Vec<ClientViolation> {
    let mut all: Vec<_> = compiled
        .metadata
        .violations
        .values()
        .flatten()
        .copied()
        .collect();
    all.sort_by_key(|v| format!("{v:?}"));
    all.dedup();
    all
}

/// The valid generator sends the current version and only flags the subprotocol defines, on a
/// fresh connection each time.
#[test]
fn the_valid_generator_conforms() {
    for seed in 0..50u64 {
        let program = generate(&SetupConnectionGenerator::new(0), seed, 2);
        let compiled = compile(&program);
        assert_eq!(
            connects(&program),
            2,
            "seed {seed}: each setup on its own connection"
        );
        assert!(compiled.metadata.violations.is_empty(), "seed {seed}");
        for setup in setups(&compiled) {
            assert_eq!(setup.min_version, CURRENT_VERSION);
            assert_eq!(setup.max_version, CURRENT_VERSION);
            assert_eq!(
                setup.flags & !defined_flags(setup.protocol),
                0,
                "seed {seed}: undefined flag in {setup:?}"
            );
        }
    }
}

/// Reuse is a choice, and a second setup on a reused connection is classified as such.
#[test]
fn connection_reuse_is_explicit() {
    let reuse = SetupConnectionGenerator {
        role: 0,
        protocol: None,
        connection: ConnectionChoice::Reuse,
    };
    let program = generate(&reuse, 3, 2);
    assert_eq!(
        connects(&program),
        1,
        "the second setup reused the first connection"
    );
    assert_eq!(
        violations(&compile(&program)),
        vec![ClientViolation::SetupNotFirst]
    );
}

/// Each adversarial violation produces exactly the departure it names, and no other.
#[test]
fn each_adversarial_violation_is_exactly_one() {
    for violation in SetupViolation::ALL {
        for seed in 0..20u64 {
            let generator = AdversarialSetupGenerator {
                role: 0,
                protocol: Some(Protocol::Mining),
                violation,
            };
            let program = generate(&generator, seed, 1);
            let compiled = compile(&program);
            let setups = setups(&compiled);
            let first = &setups[0];
            let what = format!("{violation:?} seed {seed}: {first:?}");

            match violation {
                SetupViolation::EmptyVersionRange => {
                    assert!(first.min_version > first.max_version, "{what}");
                    assert_eq!(first.flags & !defined_flags(Protocol::Mining), 0, "{what}");
                    assert!(compiled.metadata.violations.is_empty(), "{what}");
                }
                SetupViolation::UnknownVersion => {
                    assert!(
                        !(first.min_version..=first.max_version).contains(&CURRENT_VERSION),
                        "{what}"
                    );
                    assert!(compiled.metadata.violations.is_empty(), "{what}");
                }
                SetupViolation::UndefinedFlags => {
                    assert_ne!(first.flags & !defined_flags(Protocol::Mining), 0, "{what}");
                    assert_eq!(first.min_version, CURRENT_VERSION, "{what}");
                    assert!(compiled.metadata.violations.is_empty(), "{what}");
                }
                SetupViolation::AfterRawFrame => {
                    assert_eq!(
                        violations(&compiled),
                        vec![
                            ClientViolation::RawFrameBefore,
                            ClientViolation::SetupNotFirst
                        ],
                        "{what}"
                    );
                    assert!(
                        compiled.actions.iter().any(
                            |a| matches!(a, Action::Send { message_type, .. } if *message_type != 0)
                        ),
                        "{what}"
                    );
                }
                SetupViolation::SecondSetup => {
                    assert_eq!(setups.len(), 2, "{what}");
                    assert_eq!(connects(&program), 1, "{what}");
                    assert_eq!(
                        violations(&compiled),
                        vec![ClientViolation::SetupNotFirst],
                        "{what}"
                    );
                }
            }
        }
    }
}
