use std::time::Duration;

use rand::{RngExt, seq::IteratorRandom};

use crate::{
    Operation, Program, Protocol,
    mutators::{Mutator, MutatorError, MutatorResult},
};

/// Boundary values worth trying for the version fields.
///
/// Version 2 is the only one the specification currently allows, so the interesting cases
/// are the ones either side of it and the extremes of the field.
const VERSIONS: [u16; 6] = [0, 1, 2, 3, u16::MAX - 1, u16::MAX];

const PROTOCOLS: [Protocol; 3] = [
    Protocol::Mining,
    Protocol::JobDeclaration,
    Protocol::TemplateDistribution,
];

/// Rewrites the data an operation carries, leaving the shape of the program alone.
///
/// This is what reaches the values inside a message: flags a role does not support, a version
/// range that does not overlap, a string longer than the field allows.
#[derive(Default)]
pub struct OperationMutator;

impl<R: RngExt> Mutator<R> for OperationMutator {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult {
        let candidates: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| instruction.is_operation_mutable())
            .map(|(index, _)| index)
            .collect();

        let Some(index) = candidates.into_iter().choose(rng) else {
            return Err(MutatorError::NoMutationsAvailable);
        };

        let mutated = mutate_operation(&program.instructions[index].operation, rng);
        if mutated == program.instructions[index].operation {
            return Err(MutatorError::NoMutationsAvailable);
        }
        program.instructions[index].operation = mutated;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "OperationMutator"
    }
}

fn mutate_operation<R: RngExt>(operation: &Operation, rng: &mut R) -> Operation {
    match operation {
        Operation::LoadVersion(_) => {
            Operation::LoadVersion(*VERSIONS.iter().choose(rng).expect("VERSIONS is not empty"))
        }

        // Flipping a single bit keeps a mutation close to the value it came from, which is
        // what makes an unsupported flag reachable from a supported one.
        Operation::LoadFlags(flags) => Operation::LoadFlags(match rng.random_range(0..3u8) {
            0 => flags ^ (1 << rng.random_range(0..32u32)),
            1 => rng.random(),
            _ => 0,
        }),

        Operation::LoadPort(_) => Operation::LoadPort(rng.random()),
        Operation::LoadRole(role) => Operation::LoadRole(role.wrapping_add(1)),
        Operation::LoadConnection(c) => Operation::LoadConnection(c.wrapping_add(1)),

        Operation::LoadStr(value) => Operation::LoadStr(mutate_string(value, rng)),
        Operation::LoadBytes(value) => Operation::LoadBytes(mutate_bytes(value, rng)),

        Operation::LoadDuration(_) => {
            Operation::LoadDuration(Duration::from_millis(rng.random_range(0..60_000)))
        }

        Operation::EndBuildSetupConnection { .. } => Operation::EndBuildSetupConnection {
            protocol: *PROTOCOLS
                .iter()
                .choose(rng)
                .expect("PROTOCOLS is not empty"),
        },
        Operation::SendSetupConnection { .. } => Operation::SendSetupConnection {
            protocol: *PROTOCOLS
                .iter()
                .choose(rng)
                .expect("PROTOCOLS is not empty"),
        },

        Operation::SendRawFrame {
            message_type,
            extension_type,
        } => {
            if rng.random_bool(0.5) {
                Operation::SendRawFrame {
                    message_type: rng.random(),
                    extension_type: *extension_type,
                }
            } else {
                Operation::SendRawFrame {
                    message_type: *message_type,
                    extension_type: rng.random(),
                }
            }
        }

        other => other.clone(),
    }
}

/// The string fields are `STR0_255`, so the lengths worth reaching are 0, 255 and 256.
fn mutate_string<R: RngExt>(value: &str, rng: &mut R) -> String {
    match rng.random_range(0..4u8) {
        0 => String::new(),
        1 => "a".repeat(255),
        2 => "a".repeat(256),
        _ => {
            let mut mutated = value.to_string();
            mutated.push(char::from(rng.random_range(b'a'..=b'z')));
            mutated
        }
    }
}

fn mutate_bytes<R: RngExt>(value: &[u8], rng: &mut R) -> Vec<u8> {
    let mut mutated = value.to_vec();
    match rng.random_range(0..3u8) {
        0 if !mutated.is_empty() => {
            let at = rng.random_range(0..mutated.len());
            mutated[at] ^= 1 << rng.random_range(0..8u32);
        }
        1 => mutated.push(rng.random()),
        _ => {
            mutated.pop();
        }
    }
    mutated
}
