pub mod mining_channel;
pub mod raw_frame;
pub mod setup_connection;

use rand::RngExt;

use crate::{ProgramBuilder, errors::ProgramValidationError};

/// Appends a self contained fragment to a program under construction.
///
/// A generator may only append instructions the builder accepts, so anything it produces is
/// statically valid by construction.
pub trait Generator<R: RngExt> {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), ProgramValidationError>;
}
