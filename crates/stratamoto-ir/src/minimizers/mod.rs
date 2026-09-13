pub mod block;
pub mod cutting;
pub mod nopping;

use crate::Program;

/// Proposes smaller programs, one at a time.
///
/// The caller runs each candidate and reports back whether it still reproduces the failure,
/// so the minimizer can keep the reduction or put it back.
pub trait Minimizer: Iterator<Item = Program> {
    fn new(program: Program) -> Self;
    /// The last candidate still reproduced the failure.
    fn success(&mut self);
    /// The last candidate did not reproduce the failure.
    fn failure(&mut self);
    /// The smallest program found so far.
    fn current(&self) -> &Program;
}
