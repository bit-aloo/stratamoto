pub mod builder;
pub mod compiler;
pub mod errors;
pub mod generators;
pub mod instruction;
pub mod mutators;
pub mod operation;
pub mod variable;

pub use builder::{IndexedVariable, ProgramBuilder};
pub use errors::ProgramValidationError;
pub use instruction::{Instruction, InstructionContext};
pub use operation::Operation;
pub use variable::{Protocol, Variable};

use std::fmt;

use serde::{Deserialize, Serialize};

/// A sequence of operations to perform on a deployment.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Program {
    pub context: ProgramContext,
    pub instructions: Vec<Instruction>,
}

/// The state a program is executed against.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProgramContext {
    /// Number of roles in the deployment.
    pub num_roles: usize,
    /// Number of connections that exist before the program runs.
    pub num_connections: usize,
    /// Seed the deployment's simulator is initialized with.
    pub seed: u64,
}

impl Program {
    #[must_use]
    pub fn unchecked_new(context: ProgramContext, instructions: Vec<Instruction>) -> Self {
        Self {
            context,
            instructions,
        }
    }

    #[must_use]
    pub fn is_statically_valid(&self) -> bool {
        ProgramBuilder::from_program(self.clone())
            .and_then(ProgramBuilder::finalize)
            .is_ok()
    }

    pub fn to_builder(&self) -> Result<ProgramBuilder, ProgramValidationError> {
        ProgramBuilder::from_program(self.clone())
    }

    /// Drop nop instructions, renumbering the inputs that referred to later variables.
    pub fn remove_nops(&mut self) {
        let remove: Vec<bool> = self
            .instructions
            .iter()
            .map(|i| matches!(i.operation, Operation::Nop { .. }))
            .collect();
        *self = self.without(&remove);
    }

    /// Drop the marked instructions, renumbering the inputs of those that remain.
    ///
    /// Inputs that referred to a dropped instruction's variables are left pointing at
    /// whatever took their place, so the result still has to be validated.
    #[must_use]
    pub fn without(&self, remove: &[bool]) -> Program {
        let mut mapping = Vec::new();
        let mut kept = 0;

        for (index, instruction) in self.instructions.iter().enumerate() {
            let outputs =
                instruction.operation.num_outputs() + instruction.operation.num_inner_outputs();
            let dropped = remove.get(index).copied().unwrap_or(false);
            for _ in 0..outputs {
                mapping.push(kept);
                if !dropped {
                    kept += 1;
                }
            }
        }

        let instructions = self
            .instructions
            .iter()
            .enumerate()
            .filter(|(index, _)| !remove.get(*index).copied().unwrap_or(false))
            .map(|(_, instruction)| {
                let mut instruction = instruction.clone();
                for input in &mut instruction.inputs {
                    *input = mapping[*input];
                }
                instruction
            })
            .collect();

        Program {
            context: self.context.clone(),
            instructions,
        }
    }
}

impl fmt::Display for Program {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "// roles={} connections={} seed={}",
            self.context.num_roles, self.context.num_connections, self.context.seed
        )?;

        let mut var = 0;
        let mut indent = 0usize;

        for instruction in &self.instructions {
            if instruction.operation.is_block_end() {
                indent = indent.saturating_sub(1);
            }
            write!(f, "{}", "  ".repeat(indent))?;

            let outputs = instruction.operation.num_outputs();
            if outputs > 0 {
                let names: Vec<_> = (var..var + outputs).map(|i| format!("v{i}")).collect();
                var += outputs;
                write!(f, "{} <- ", names.join(", "))?;
            }

            write!(f, "{}", instruction.operation)?;

            if !instruction.inputs.is_empty() {
                let names: Vec<_> = instruction.inputs.iter().map(|i| format!("v{i}")).collect();
                write!(f, "({})", names.join(", "))?;
            }

            let inner = instruction.operation.num_inner_outputs();
            if inner > 0 {
                let names: Vec<_> = (var..var + inner).map(|i| format!("v{i}")).collect();
                var += inner;
                write!(f, " -> {}", names.join(", "))?;
            }

            writeln!(f)?;

            if instruction.operation.is_block_begin() {
                indent += 1;
            }
        }
        Ok(())
    }
}
