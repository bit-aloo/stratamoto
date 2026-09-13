use rand::{RngExt, seq::IndexedRandom};

use crate::{
    IndexedVariable, Operation, ProgramBuilder, Protocol, Variable,
    errors::ProgramValidationError, generators::Generator,
};

const PROTOCOLS: [Protocol; 3] = [
    Protocol::Mining,
    Protocol::JobDeclaration,
    Protocol::TemplateDistribution,
];

/// Opens a connection between two roles and sets it up for a subprotocol.
///
/// The session it produces is what later subprotocol messages consume, so this is the
/// fragment every mining, job declaration or template distribution program starts from.
pub struct SetupConnectionGenerator {
    /// The role the connection is opened to.
    pub role: usize,
}

impl<R: RngExt> Generator<R> for SetupConnectionGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), ProgramValidationError> {
        let protocol = *PROTOCOLS.choose(rng).expect("PROTOCOLS is not empty");

        let connection = match builder.get_random_variable(rng, &Variable::Connection) {
            Some(connection) => connection,
            None => {
                let role = one(builder.append_op(Operation::LoadRole(self.role), &[])?);
                one(builder.append_op(Operation::Connect, &[&role])?)
            }
        };

        let min_version = one(builder.append_op(Operation::LoadVersion(2), &[])?);
        let max_version = one(builder.append_op(Operation::LoadVersion(2), &[])?);
        let flags = one(builder.append_op(Operation::LoadFlags(rng.random()), &[])?);

        let setup = one(builder.append_op(Operation::BeginBuildSetupConnection, &[])?);
        builder.append_op(Operation::SetVersions, &[&setup, &min_version, &max_version])?;
        builder.append_op(Operation::SetFlags, &[&setup, &flags])?;
        let setup = one(builder.append_op(
            Operation::EndBuildSetupConnection { protocol },
            &[&setup],
        )?);

        builder.append_op(
            Operation::SendSetupConnection { protocol },
            &[&connection, &setup],
        )?;

        Ok(())
    }
}

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    assert_eq!(variables.len(), 1, "operation produces exactly one variable");
    variables.pop().expect("checked above")
}
