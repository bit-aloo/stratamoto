use rand::RngExt;

use crate::{
    Program, ProgramBuilder,
    mutators::{Mutator, MutatorError, MutatorResult, Splicer},
};

/// Appends another program's instructions, renumbering their inputs.
///
/// Concatenation is how a program grows past what a single generator produces: two setups on
/// one deployment, or a setup followed by whatever a second program did.
#[derive(Default)]
pub struct ConcatMutator;

impl<R: RngExt> Mutator<R> for ConcatMutator {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult {
        let other = program.clone();
        self.splice(program, &other, rng)
    }

    fn name(&self) -> &'static str {
        "ConcatMutator"
    }
}

impl<R: RngExt> Splicer<R> for ConcatMutator {
    fn splice(&mut self, program: &mut Program, other: &Program, _rng: &mut R) -> MutatorResult {
        if other.instructions.is_empty() {
            return Err(MutatorError::NoMutationsAvailable);
        }

        // The appended instructions refer to their own variables, which sit after the ones
        // already in the program.
        let offset = variable_count(program);
        let mut appended = other.instructions.clone();
        for instruction in &mut appended {
            for input in &mut instruction.inputs {
                *input += offset;
            }
        }

        let mut instructions = program.instructions.clone();
        instructions.extend(appended);

        let context = program.context.clone();
        ProgramBuilder::from_prefix(context, &instructions)
            .and_then(ProgramBuilder::finalize)
            .map_err(|_| MutatorError::CreatedInvalidProgram)?;

        program.instructions = instructions;
        Ok(())
    }
}

fn variable_count(program: &Program) -> usize {
    program
        .instructions
        .iter()
        .map(|i| i.operation.num_outputs() + i.operation.num_inner_outputs())
        .sum()
}
