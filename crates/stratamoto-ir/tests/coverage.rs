use std::collections::BTreeSet;

use rand::{RngExt, SeedableRng, rngs::SmallRng};
use stratamoto_ir::{
    Program, ProgramBuilder, ProgramContext,
    compiler::{Compiler, SetupConnectionSpec},
    generators::{Generator, setup_connection::SetupConnectionGenerator},
    mutators::{
        Mutator, concat::ConcatMutator, generate::GeneratorMutator, input::InputMutator,
        operation::OperationMutator,
    },
};

const ROLES: usize = 3;

/// Every distinct `SetupConnection` a campaign puts on the wire.
fn reached(rounds: usize) -> Vec<SetupConnectionSpec> {
    let context = ProgramContext {
        num_roles: ROLES,
        num_connections: 0,
        seed: 0,
    };
    let mut rng = SmallRng::seed_from_u64(1);

    let mut builder = ProgramBuilder::new(context.clone());
    SetupConnectionGenerator { role: 0 }
        .generate(&mut builder, &mut rng)
        .unwrap();
    let mut corpus: Vec<Program> = vec![builder.finalize().unwrap()];

    let mut specs = Vec::new();

    for _ in 0..rounds {
        let pick = rng.random_range(0..corpus.len());
        let mut program = corpus[pick].clone();

        let mut applied = false;
        for _ in 0..rng.random_range(1..=4u32) {
            let role = rng.random_range(0..ROLES);
            let result = match rng.random_range(0..4u8) {
                0 => InputMutator.mutate(&mut program, &mut rng),
                1 => OperationMutator.mutate(&mut program, &mut rng),
                2 => ConcatMutator.mutate(&mut program, &mut rng),
                _ => GeneratorMutator::new(SetupConnectionGenerator { role })
                    .mutate(&mut program, &mut rng),
            };
            applied |= result.is_ok();
        }
        if !applied || program.instructions.len() > 256 || !program.is_statically_valid() {
            continue;
        }

        if let Ok(compiled) = Compiler::new().compile(&program) {
            specs.extend(compiled.metadata.session_setups.values().cloned());
        }
        if corpus.len() < 200 {
            corpus.push(program);
        }
    }

    specs
}

/// A field no operation writes is a field no mutation can reach, so every field of the message
/// has to be reachable through some generator. This is what caught endpoint and device
/// information never varying at all.
#[test]
fn mutation_reaches_every_setup_connection_field() {
    let specs = reached(5_000);
    assert!(!specs.is_empty(), "no programs compiled");

    let distinct = |f: fn(&SetupConnectionSpec) -> String| {
        specs.iter().map(f).collect::<BTreeSet<_>>().len()
    };

    for (field, count) in [
        ("protocol", distinct(|s| format!("{:?}", s.protocol))),
        ("min_version", distinct(|s| s.min_version.to_string())),
        ("max_version", distinct(|s| s.max_version.to_string())),
        ("flags", distinct(|s| s.flags.to_string())),
        ("endpoint_host", distinct(|s| s.endpoint_host.clone())),
        ("endpoint_port", distinct(|s| s.endpoint_port.to_string())),
        ("vendor", distinct(|s| s.vendor.clone())),
        ("hardware_version", distinct(|s| s.hardware_version.clone())),
        ("firmware", distinct(|s| s.firmware.clone())),
        ("device_id", distinct(|s| s.device_id.clone())),
    ] {
        assert!(count > 1, "{field} never varied: {count} distinct value");
    }
}
