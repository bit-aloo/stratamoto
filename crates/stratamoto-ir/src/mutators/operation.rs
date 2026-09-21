use std::time::Duration;

use rand::{
    RngExt,
    seq::{IndexedRandom, SliceRandom},
};

use crate::{
    Operation, Program, ProgramContext,
    mutators::{Mutator, MutatorError, MutatorResult},
};

/// Boundary values worth trying for the version fields.
///
/// Version 2 is the only one the specification currently allows, so the interesting cases
/// are the ones either side of it and the extremes of the field.
const VERSIONS: [u16; 6] = [0, 1, 2, 3, u16::MAX - 1, u16::MAX];

/// The flags the mining and job declaration setups define, all of them together, the first
/// bit none defines, the sign bit and every bit.
const FLAGS: [u32; 8] = [0, 1, 2, 4, 0b111, 0b1000, 0x8000_0000, u32::MAX];

/// Either side of the privileged range, and the extremes of the field.
const PORTS: [u16; 6] = [0, 1, 1023, 1024, u16::MAX - 1, u16::MAX];

/// The setup messages, the reconnect, and the extremes of the message type space.
const MESSAGE_TYPES: [u8; 8] = [0x00, 0x01, 0x02, 0x03, 0x04, 0x10, 0x7f, 0xff];

/// No extension, the first one, and the values either side of the channel bit.
const EXTENSION_TYPES: [u16; 5] = [0, 1, 0x7fff, 0x8000, u16::MAX];

const DURATIONS: [Duration; 6] = [
    Duration::ZERO,
    Duration::from_millis(1),
    Duration::from_secs(1),
    Duration::from_secs(10),
    Duration::from_secs(60),
    Duration::from_secs(3600),
];

/// Rewrites the data an operation carries, leaving the shape of the program alone.
///
/// This is what reaches the values inside a message: flags a role does not support, a version
/// range that does not overlap, a string longer than the field allows.
#[derive(Default)]
pub struct OperationMutator;

impl<R: RngExt> Mutator<R> for OperationMutator {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult {
        let context = &program.context;
        let instructions = &mut program.instructions;

        // Which operations carry data is known to `mutate_operation` alone. Rather than keep a
        // second list of them here to fall out of step with it, try the instructions in random
        // order until one yields a rewrite, which picks uniformly among the ones that do.
        let mut order: Vec<usize> = (0..instructions.len()).collect();
        order.shuffle(rng);

        for index in order {
            let instruction = &mut instructions[index];
            if let Some(mutated) = mutate_operation(&instruction.operation, context, rng) {
                debug_assert_ne!(
                    mutated, instruction.operation,
                    "a rewrite has to change the operation"
                );
                instruction.operation = mutated;
                return Ok(());
            }
        }

        Err(MutatorError::NoMutationsAvailable)
    }

    fn name(&self) -> &'static str {
        "OperationMutator"
    }
}

/// The rewrite an operation admits, or `None` for one that carries nothing to rewrite.
///
/// The match is exhaustive on purpose: an operation added without deciding whether it is
/// mutable does not compile, and there is no second list elsewhere for this one to drift from.
/// Every rewrite yields an operation different from the one it was given.
fn mutate_operation<R: RngExt>(
    operation: &Operation,
    context: &ProgramContext,
    rng: &mut R,
) -> Option<Operation> {
    Some(match operation {
        // A role or connection is only worth rewriting to another the program could name,
        // since the builder rejects any other.
        Operation::LoadRole(role) => {
            Operation::LoadRole(other_index(*role, context.num_roles, rng)?)
        }
        Operation::LoadConnection(connection) => {
            Operation::LoadConnection(other_index(*connection, context.num_connections, rng)?)
        }

        Operation::LoadVersion(value) => Operation::LoadVersion(mutate_u16(*value, &VERSIONS, rng)),
        Operation::LoadFlags(value) => Operation::LoadFlags(mutate_u32(*value, &FLAGS, rng)),
        Operation::LoadPort(value) => Operation::LoadPort(mutate_u16(*value, &PORTS, rng)),
        Operation::LoadStr(value) => Operation::LoadStr(mutate_string(value, rng)),
        Operation::LoadBytes(value) => Operation::LoadBytes(mutate_bytes(value, rng)),
        Operation::LoadDuration(value) => Operation::LoadDuration(mutate_duration(*value, rng)),

        Operation::SendRawFrame {
            message_type,
            extension_type,
        } => {
            if rng.random_bool(0.5) {
                Operation::SendRawFrame {
                    message_type: mutate_u8(*message_type, &MESSAGE_TYPES, rng),
                    extension_type: *extension_type,
                }
            } else {
                Operation::SendRawFrame {
                    message_type: *message_type,
                    extension_type: mutate_u16(*extension_type, &EXTENSION_TYPES, rng),
                }
            }
        }

        // The protocol of a setup is written on the end of the block that builds it, on the
        // send, and on the block that waits for its success, and they have to agree for the
        // program to type-check. Rewriting one alone can only produce a program the builder
        // rejects; changing them together is a structural mutation, not a rewrite of one
        // operation.
        Operation::EndBuildSetupConnection { .. }
        | Operation::SendSetupConnection { .. }
        | Operation::BeginOnSetupSuccess { .. } => return None,

        Operation::Nop { .. }
        | Operation::Connect
        | Operation::BeginBuildSetupConnection
        | Operation::EndOnSetupSuccess
        | Operation::SetVersions
        | Operation::SetFlags
        | Operation::SetEndpoint
        | Operation::SetDeviceInfo
        | Operation::AdvanceTime
        | Operation::Probe => return None,
    })
}

/// Another index below `count`, or `None` when there is no other.
///
/// An index that is already out of range can go anywhere in range, since anywhere is a change.
fn other_index<R: RngExt>(current: usize, count: usize, rng: &mut R) -> Option<usize> {
    if count == 0 {
        return None;
    }
    if current >= count {
        return Some(rng.random_range(0..count));
    }
    if count == 1 {
        return None;
    }
    let pick = rng.random_range(0..count - 1);
    Some(if pick >= current { pick + 1 } else { pick })
}

macro_rules! word_mutation {
    ($name:ident, $ty:ty) => {
        /// A boundary value, the value with one bit flipped, or a random one; never the value
        /// given.
        ///
        /// Flipping a bit keeps a mutation close to the value it came from, which is what makes
        /// an unsupported flag reachable from a supported one.
        fn $name<R: RngExt>(value: $ty, edges: &[$ty], rng: &mut R) -> $ty {
            loop {
                let candidate = match rng.random_range(0..3u8) {
                    0 => *edges.choose(rng).expect("every edge set has a value"),
                    1 => value ^ (1 << rng.random_range(0..<$ty>::BITS)),
                    _ => rng.random(),
                };
                if candidate != value {
                    return candidate;
                }
            }
        }
    };
}

word_mutation!(mutate_u8, u8);
word_mutation!(mutate_u16, u16);
word_mutation!(mutate_u32, u32);

/// The string fields are `STR0_255`, so the lengths worth reaching are 0, 255 and 256.
fn mutate_string<R: RngExt>(value: &str, rng: &mut R) -> String {
    loop {
        let candidate = match rng.random_range(0..4u8) {
            0 => String::new(),
            1 => "a".repeat(255),
            2 => "a".repeat(256),
            _ => {
                let mut mutated = value.to_string();
                mutated.push(char::from(rng.random_range(b'a'..=b'z')));
                mutated
            }
        };
        if candidate != value {
            return candidate;
        }
    }
}

/// One bit flipped, one byte more, or one byte fewer.
fn mutate_bytes<R: RngExt>(value: &[u8], rng: &mut R) -> Vec<u8> {
    loop {
        let mut candidate = value.to_vec();
        match rng.random_range(0..3u8) {
            0 if !candidate.is_empty() => {
                let at = rng.random_range(0..candidate.len());
                candidate[at] ^= 1 << rng.random_range(0..8u32);
            }
            1 => candidate.push(rng.random()),
            _ => {
                candidate.pop();
            }
        }
        if candidate != value {
            return candidate;
        }
    }
}

fn mutate_duration<R: RngExt>(value: Duration, rng: &mut R) -> Duration {
    loop {
        let candidate = if rng.random_bool(0.5) {
            *DURATIONS.choose(rng).expect("DURATIONS is not empty")
        } else {
            Duration::from_millis(rng.random_range(0..60_000))
        };
        if candidate != value {
            return candidate;
        }
    }
}
