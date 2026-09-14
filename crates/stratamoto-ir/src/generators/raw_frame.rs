use rand::RngExt;

use crate::{
    IndexedVariable, Operation, ProgramBuilder, Variable, errors::ProgramValidationError,
    generators::Generator,
};

/// Sends arbitrary bytes under an arbitrary message type on a connection.
///
/// Every other operation builds a message the encoder will accept, which is what a conformant
/// client could have sent. This is the way out of that: a frame whose payload never had to
/// parse, so a role's decoder is reached with bytes no message type would produce.
pub struct RawFrameGenerator {
    /// The role a connection is opened to, if the program has none to reuse.
    pub role: usize,
}

impl<R: RngExt> Generator<R> for RawFrameGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), ProgramValidationError> {
        let connection = match builder.get_random_variable(rng, &Variable::Connection) {
            Some(connection) => connection,
            None => {
                let role = one(builder.append_op(Operation::LoadRole(self.role), &[])?);
                one(builder.append_op(Operation::Connect, &[&role])?)
            }
        };

        let length = rng.random_range(0..64usize);
        let payload: Vec<u8> = (0..length).map(|_| rng.random()).collect();
        let bytes = one(builder.append_op(Operation::LoadBytes(payload), &[])?);

        builder.append_op(
            Operation::SendRawFrame {
                message_type: rng.random(),
                extension_type: 0,
            },
            &[&connection, &bytes],
        )?;

        Ok(())
    }
}

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    assert_eq!(variables.len(), 1, "operation produces exactly one variable");
    variables.pop().expect("checked above")
}
