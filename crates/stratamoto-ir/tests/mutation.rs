use rand::{SeedableRng, rngs::SmallRng};
use stratamoto_ir::{
    Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    generators::{Generator, setup_connection::SetupConnectionGenerator},
    minimizers::{Minimizer, block::BlockMinimizer, cutting::CuttingMinimizer,
                 nopping::NoppingMinimizer},
    mutators::{Mutator, concat::ConcatMutator, generate::GeneratorMutator, input::InputMutator,
               operation::OperationMutator},
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
    SetupConnectionGenerator { role: 0 }
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
    let mut generate = GeneratorMutator::new(SetupConnectionGenerator { role: 1 });

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

#[test]
fn the_operation_mutator_reaches_every_protocol() {
    let mut mutator = OperationMutator;
    let mut seen = vec![];

    for seed in 0..100u64 {
        let mut rng = SmallRng::seed_from_u64(seed);
        let mut program = seeded(seed);
        let _ = mutator.mutate(&mut program, &mut rng);

        for instruction in &program.instructions {
            if let Operation::EndBuildSetupConnection { protocol } = instruction.operation
                && !seen.contains(&protocol)
            {
                seen.push(protocol);
            }
        }
    }

    for protocol in [
        Protocol::Mining,
        Protocol::JobDeclaration,
        Protocol::TemplateDistribution,
    ] {
        assert!(seen.contains(&protocol), "never reached {protocol}");
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
    assert!(
        program.instructions.is_empty(),
        "left over:\n{program}"
    );
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
    assert!(program.instructions.len() >= 5, "over minimized:\n{program}");
}
