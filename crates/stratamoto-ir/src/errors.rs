use std::fmt;

use crate::{Operation, Variable};

#[derive(Debug, Clone, PartialEq)]
pub enum ProgramValidationError {
    InvalidNumberOfInputs { is: usize, expected: usize },
    InvalidVariableType { is: Variable, expected: Variable },
    VariableNotDefined(usize),
    RoleNotFound(usize),
    ConnectionNotFound(usize),
    InvalidBlockEnd { begin: Operation, end: Operation },
    UnfinishedBlock(Operation),
}

impl fmt::Display for ProgramValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgramValidationError::InvalidNumberOfInputs { is, expected } => {
                write!(f, "expected {expected} inputs, got {is}")
            }
            ProgramValidationError::InvalidVariableType { is, expected } => {
                write!(f, "expected a {expected} input, got a {is}")
            }
            ProgramValidationError::VariableNotDefined(i) => write!(f, "v{i} is not in scope"),
            ProgramValidationError::RoleNotFound(i) => write!(f, "role {i} is not in the context"),
            ProgramValidationError::ConnectionNotFound(i) => {
                write!(f, "connection {i} is not in the context")
            }
            ProgramValidationError::InvalidBlockEnd { begin, end } => {
                write!(f, "{end} does not close {begin}")
            }
            ProgramValidationError::UnfinishedBlock(op) => write!(f, "{op} is never closed"),
        }
    }
}

impl std::error::Error for ProgramValidationError {}
