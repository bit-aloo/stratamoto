use std::{mem::discriminant, time::Duration};

use rand::{SeedableRng, rngs::SmallRng};
use stratamoto_ir::{
    Instruction, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    generators::{Generator, setup_connection::SetupConnectionGenerator},
    minimizers::{
        Minimizer, block::BlockMinimizer, cutting::CuttingMinimizer, nopping::NoppingMinimizer,
    },
    mutators::{
        Mutator, MutatorError, concat::ConcatMutator, generate::GeneratorMutator,
        input::InputMutator, operation::OperationMutator,
    },
};

fn context() -> ProgramContext {
    ProgramContext {
        num_roles: 3,
        num_connections: 0,
        seed: 0,
    }
}

fn seeded(seed: u64) -> Program {
    let mut rng = SmallRng::seed_from_u64(seed);
    let mut builder = ProgramBuilder::new(context());
    SetupConnectionGenerator::new(0)
        .generate(&mut builder, &mut rng)
        .unwrap();
    builder.finalize().unwrap()
}

/// A mutator may fail to find a mutation, but anything it does produce has to be a program
/// the builder would have accepted.
#[test]
fn mutators_never_produce_an_invalid_program() {
    let mut input = InputMutator;
    let mut operation = OperationMutator;
    let mut concat = ConcatMutator;
    let mut generate = GeneratorMutator::new(SetupConnectionGenerator::new(1));

    for seed in 0..200u64 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut program = seeded(seed);

        for round in 0..8 {
            let before = program.clone();
            let result = match round % 4 {
                0 => input.mutate(&mut program, &mut rng),
                1 => operation.mutate(&mut program, &mut rng),
                2 => concat.mutate(&mut program, &mut rng),
                _ => generate.mutate(&mut program, &mut rng),
            };

            if result.is_ok() {
                assert!(
                    program.is_statically_valid(),
                    "seed {seed} round {round} produced:\n{program}from:\n{before}"
                );
            }
        }
    }
}

/// A program of one instruction and nothing else, so a rewrite has exactly one place to land.
fn single(operation: Operation) -> Program {
    let context = ProgramContext {
        num_roles: 3,
        num_connections: 2,
        seed: 0,
    };
    Program::unchecked_new(context, vec![Instruction::new(operation, vec![])])
}

/// What the one instruction became on each of `runs` seeds, or `None` where the mutator
/// declined.
fn rewrites(operation: &Operation, runs: u64) -> Vec<Option<Operation>> {
    (0..runs)
        .map(|seed| {
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut program = single(operation.clone());
            match OperationMutator.mutate(&mut program, &mut rng) {
                Ok(()) => {
                    assert_eq!(program.instructions.len(), 1, "the shape changed");
                    assert!(
                        program.instructions[0].inputs.is_empty(),
                        "the inputs changed"
                    );
                    Some(program.instructions[0].operation.clone())
                }
                Err(MutatorError::NoMutationsAvailable) => {
                    assert_eq!(program, single(operation.clone()), "declined but changed");
                    None
                }
                Err(e) => panic!("unexpected {e:?}"),
            }
        })
        .collect()
}

/// Every operation that carries data is rewritten, to the same kind of operation with a
/// different value, every time; every other operation is declined, every time.
#[test]
fn every_operation_is_rewritten_or_declined_consistently() {
    let mutable = [
        Operation::LoadRole(0),
        Operation::LoadConnection(0),
        Operation::LoadVersion(2),
        Operation::LoadFlags(0),
        Operation::LoadPort(0),
        Operation::LoadStr(String::new()),
        Operation::LoadBytes(vec![]),
        Operation::LoadDuration(Duration::ZERO),
        Operation::SendRawFrame {
            message_type: 0,
            extension_type: 0,
        },
    ];
    let immutable = [
        Operation::Nop {
            outputs: 0,
            inner_outputs: 0,
        },
        Operation::Connect,
        Operation::BeginBuildSetupConnection,
        Operation::SetVersions,
        Operation::SetFlags,
        Operation::SetEndpoint,
        Operation::SetDeviceInfo,
        Operation::EndBuildSetupConnection {
            protocol: Protocol::Mining,
        },
        Operation::SendSetupConnection {
            protocol: Protocol::Mining,
        },
        Operation::BeginOnSetupSuccess {
            protocol: Protocol::Mining,
        },
        Operation::EndOnSetupSuccess,
        Operation::AdvanceTime,
        Operation::Probe,
    ];

    for operation in &mutable {
        for rewritten in rewrites(operation, 50) {
            let rewritten = rewritten.unwrap_or_else(|| panic!("declined {operation}"));
            assert_eq!(
                discriminant(&rewritten),
                discriminant(operation),
                "{operation} became {rewritten}"
            );
            assert_ne!(&rewritten, operation, "{operation} was rewritten to itself");
        }
    }

    for operation in &immutable {
        for rewritten in rewrites(operation, 50) {
            assert!(rewritten.is_none(), "{operation} was rewritten");
        }
    }
}

/// A role or connection is rewritten only to another one the context has.
#[test]
fn roles_and_connections_are_rewritten_within_the_context() {
    let roles: Vec<usize> = rewrites(&Operation::LoadRole(1), 100)
        .into_iter()
        .map(|op| match op {
            Some(Operation::LoadRole(role)) => role,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(roles.contains(&0) && roles.contains(&2), "{roles:?}");
    assert!(roles.iter().all(|&r| r == 0 || r == 2), "{roles:?}");

    let connections: Vec<usize> = rewrites(&Operation::LoadConnection(0), 100)
        .into_iter()
        .map(|op| match op {
            Some(Operation::LoadConnection(c)) => c,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(connections.iter().all(|&c| c == 1), "{connections:?}");

    // With nothing else to name, there is nothing to rewrite.
    let alone = ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    };
    let mut program = Program::unchecked_new(
        alone,
        vec![Instruction::new(Operation::LoadRole(0), vec![])],
    );
    let mut rng = SmallRng::seed_from_u64(0);
    assert_eq!(
        OperationMutator.mutate(&mut program, &mut rng),
        Err(MutatorError::NoMutationsAvailable)
    );
}

/// Each field's rewrite reaches the boundary values it is meant to, not only random ones.
#[test]
fn rewrites_reach_the_boundaries_of_each_field() {
    const RUNS: u64 = 600;

    fn reached<T: PartialEq + std::fmt::Debug>(
        operation: Operation,
        extract: impl Fn(Operation) -> T,
        expected: &[T],
    ) {
        let values: Vec<T> = rewrites(&operation, RUNS)
            .into_iter()
            .map(|op| extract(op.expect("a mutable operation is always rewritten")))
            .collect();
        for value in expected {
            assert!(
                values.contains(value),
                "{operation} never reached {value:?}; reached {values:?}"
            );
        }
    }

    macro_rules! field {
        ($variant:ident) => {
            |op| match op {
                Operation::$variant(v) => v,
                other => panic!("{other}"),
            }
        };
    }

    reached(
        Operation::LoadVersion(2),
        field!(LoadVersion),
        &[0, 1, 3, u16::MAX],
    );
    reached(
        Operation::LoadFlags(0b111),
        field!(LoadFlags),
        &[0, 0b1000, 0x8000_0000, u32::MAX],
    );
    reached(
        Operation::LoadPort(3333),
        field!(LoadPort),
        &[0, 1024, u16::MAX],
    );
    reached(
        Operation::LoadStr("sri".to_string()),
        |op| match op {
            Operation::LoadStr(s) => s.len(),
            other => panic!("{other}"),
        },
        &[0, 4, 255, 256],
    );
    reached(
        Operation::LoadBytes(vec![1, 2, 3]),
        |op| match op {
            Operation::LoadBytes(b) => b.len(),
            other => panic!("{other}"),
        },
        &[2, 3, 4],
    );
    reached(
        Operation::LoadDuration(Duration::from_secs(1)),
        field!(LoadDuration),
        &[Duration::ZERO, Duration::from_secs(3600)],
    );
    reached(
        Operation::SendRawFrame {
            message_type: 0,
            extension_type: 0,
        },
        |op| match op {
            Operation::SendRawFrame {
                message_type,
                extension_type,
            } => (message_type, extension_type),
            other => panic!("{other}"),
        },
        &[(0xff, 0), (0x10, 0), (0, 0x8000), (0, u16::MAX)],
    );
}

/// On a generated program, a rewrite changes the value of exactly one instruction and leaves
/// the program valid.
#[test]
fn a_rewrite_changes_one_operation_and_nothing_else() {
    for seed in 0..100u64 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let original = seeded(seed);
        let mut program = original.clone();
        OperationMutator
            .mutate(&mut program, &mut rng)
            .expect("a setup carries data to rewrite");

        assert_eq!(program.context, original.context);
        assert_eq!(program.instructions.len(), original.instructions.len());
        let changed: Vec<usize> = (0..program.instructions.len())
            .filter(|&i| program.instructions[i] != original.instructions[i])
            .collect();
        let [index] = changed[..] else {
            panic!("seed {seed} changed {changed:?}:\n{program}from:\n{original}");
        };
        let (before, after) = (&original.instructions[index], &program.instructions[index]);
        assert_eq!(before.inputs, after.inputs);
        assert_eq!(
            discriminant(&before.operation),
            discriminant(&after.operation)
        );
        assert!(program.is_statically_valid(), "seed {seed}:\n{program}");
    }
}

/// The protocol of a setup is written on the block end and on the send, and the two have to
/// agree, so an operation rewrite leaves it alone rather than break one of them.
#[test]
fn the_operation_mutator_leaves_the_protocol_alone() {
    let protocols = |program: &Program| -> Vec<Protocol> {
        program
            .instructions
            .iter()
            .filter_map(|i| match i.operation {
                Operation::EndBuildSetupConnection { protocol }
                | Operation::SendSetupConnection { protocol } => Some(protocol),
                _ => None,
            })
            .collect()
    };

    for seed in 0..100u64 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let original = seeded(seed);
        let mut program = original.clone();
        for _ in 0..8 {
            let _ = OperationMutator.mutate(&mut program, &mut rng);
        }
        assert_eq!(protocols(&program), protocols(&original), "seed {seed}");
    }
}

/// Minimizing against a predicate that always holds should strip the program to nothing.
#[test]
fn minimizers_reduce_a_program_that_is_never_needed() {
    let mut program = seeded(3);
    let mut rng = SmallRng::seed_from_u64(3);
    let mut concat = ConcatMutator;
    concat.mutate(&mut program, &mut rng).unwrap();
    let original = program.instructions.len();

    let mut cutting = CuttingMinimizer::new(program);
    while let Some(_candidate) = cutting.next() {
        cutting.success();
    }
    let mut program = cutting.current().clone();
    assert!(program.instructions.len() < original);

    let mut blocks = BlockMinimizer::new(program);
    while blocks.next().is_some() {
        blocks.success();
    }
    program = blocks.current().clone();

    let mut nopping = NoppingMinimizer::new(program);
    while nopping.next().is_some() {
        nopping.success();
    }
    program = nopping.current().clone();
    program.remove_nops();

    assert!(program.is_statically_valid());
    assert!(program.instructions.is_empty(), "left over:\n{program}");
}

/// Minimizing against a predicate that only the send satisfies has to keep everything the
/// send depends on.
#[test]
fn minimizers_keep_what_the_failure_needs() {
    let program = seeded(5);
    let sends = |p: &Program| {
        p.instructions
            .iter()
            .any(|i| matches!(i.operation, Operation::SendSetupConnection { .. }))
    };
    assert!(sends(&program));

    let mut nopping = NoppingMinimizer::new(program);
    while let Some(candidate) = nopping.next() {
        if sends(&candidate) {
            nopping.success();
        } else {
            nopping.failure();
        }
    }

    let mut program = nopping.current().clone();
    program.remove_nops();
    assert!(program.is_statically_valid());
    assert!(sends(&program));
    // The send needs a connection and a finalized setup connection, which need the block and
    // the role load, so nothing but the unused loads can go.
    assert!(
        program.instructions.len() >= 5,
        "over minimized:\n{program}"
    );
}
