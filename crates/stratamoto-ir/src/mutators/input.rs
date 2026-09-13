use rand::{RngExt, seq::IteratorRandom};

use crate::{
    Program, ProgramBuilder,
    mutators::{Mutator, MutatorError, MutatorResult},
};

/// Rewires one input of an instruction to another variable of the same type.
///
/// Because the replacement has to have the same type and be in scope at that point, this
/// explores how a message behaves when it refers to a different earlier value, without ever
/// producing a program that could not have been written by hand.
#[derive(Default)]
pub struct InputMutator;

impl<R: RngExt> Mutator<R> for InputMutator {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult {
        let candidates: Vec<usize> = program
            .instructions
            .iter()
            .enumerate()
            .filter(|(_, instruction)| !instruction.inputs.is_empty())
            .map(|(index, _)| index)
            .collect();

        let Some(index) = candidates.into_iter().choose(rng) else {
            return Err(MutatorError::NoMutationsAvailable);
        };

        let builder = ProgramBuilder::from_prefix(
            program.context.clone(),
            &program.instructions[..index],
        )
        .map_err(|_| MutatorError::CreatedInvalidProgram)?;

        let slot = rng.random_range(0..program.instructions[index].inputs.len());
        let current = program.instructions[index].inputs[slot];
        let Some(variable) = builder.get_variable(current) else {
            return Err(MutatorError::CreatedInvalidProgram);
        };

        let Some(replacement) = builder.get_random_variable(rng, &variable.var) else {
            return Err(MutatorError::NoMutationsAvailable);
        };
        if replacement.index == current {
            return Err(MutatorError::NoMutationsAvailable);
        }

        program.instructions[index].inputs[slot] = replacement.index;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "InputMutator"
    }
}
