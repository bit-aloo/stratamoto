use serde::{Deserialize, Serialize};

use crate::Operation;

/// An [`Operation`] together with the variables it consumes.
///
/// Inputs are indices into the list of variables produced by preceding instructions, so a
/// program is in static single assignment form.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub struct Instruction {
    pub inputs: Vec<usize>,
    pub operation: Operation,
}

impl Instruction {
    #[must_use]
    pub fn new(operation: Operation, inputs: Vec<usize>) -> Self {
        Self { inputs, operation }
    }

    /// Whether the operation can be replaced by a nop without invalidating the program.
    ///
    /// Block delimiters cannot, as their scope would be left unbalanced.
    #[must_use]
    pub fn is_noppable(&self) -> bool {
        !self.operation.is_block_begin() && !self.operation.is_block_end()
    }

    /// Whether the operation carries data a mutator may rewrite in place.
    #[must_use]
    pub fn is_operation_mutable(&self) -> bool {
        matches!(
            self.operation,
            Operation::LoadRole(_)
                | Operation::LoadConnection(_)
                | Operation::LoadVersion(_)
                | Operation::LoadFlags(_)
                | Operation::LoadPort(_)
                | Operation::LoadStr(_)
                | Operation::LoadBytes(_)
                | Operation::LoadDuration(_)
                | Operation::SendRawFrame { .. }
        )
    }

    pub fn nop(&mut self) {
        self.inputs.clear();
        self.operation = Operation::Nop {
            outputs: self.operation.num_outputs(),
            inner_outputs: self.operation.num_inner_outputs(),
        };
    }

    /// The context a block beginning opens.
    #[must_use]
    pub fn entered_context_after_execution(&self) -> Option<InstructionContext> {
        match self.operation {
            Operation::BeginBuildSetupConnection => Some(InstructionContext::BuildSetupConnection),
            _ => None,
        }
    }
}

/// The scope an [`Instruction`] is written in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstructionContext {
    Global,
    BuildSetupConnection,
}
