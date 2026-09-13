pub mod concat;
pub mod generate;
pub mod input;
pub mod operation;

use rand::RngExt;

use crate::Program;

#[derive(Debug, Clone, PartialEq)]
pub enum MutatorError {
    /// The program offers nothing for this mutator to change.
    NoMutationsAvailable,
    /// The mutation produced a program the builder rejects.
    CreatedInvalidProgram,
}

pub type MutatorResult = Result<(), MutatorError>;

/// Rewrites a program in place.
///
/// A mutator may leave the program invalid; the caller is expected to check and discard.
pub trait Mutator<R: RngExt> {
    fn mutate(&mut self, program: &mut Program, rng: &mut R) -> MutatorResult;
    fn name(&self) -> &'static str;
}

/// Mixes a second program into the one being mutated.
pub trait Splicer<R: RngExt>: Mutator<R> {
    fn splice(&mut self, program: &mut Program, other: &Program, rng: &mut R) -> MutatorResult;
}
