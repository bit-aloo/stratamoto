use rand::RngExt;

use crate::{
    Program, ProgramBuilder,
    generators::Generator,
    mutators::{Mutator, MutatorError, MutatorResult},
};

/// Appends a generated fragment to a program.
///
/// Mutating values alone can never introduce an operation the program does not already have,
/// so growth has to come from a generator.
pub struct GeneratorMutator<G> {
    pub generator: G,
}

impl<G> GeneratorMutator<G> {
    pub fn new(generator: G) -> Self {
        Self { generator }
    }
}

impl<R: RngExt, G: Generator<R>> Mutator<R> for GeneratorMutator<G> {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult {
        let mut builder =
            ProgramBuilder::from_prefix(program.context.clone(), &program.instructions)
                .map_err(|_| MutatorError::CreatedInvalidProgram)?;

        self.generator
            .generate(&mut builder, rng)
            .map_err(|_| MutatorError::CreatedInvalidProgram)?;

        *program = builder
            .finalize()
            .map_err(|_| MutatorError::CreatedInvalidProgram)?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "GeneratorMutator"
    }
}
