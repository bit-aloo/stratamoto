use std::collections::HashSet;

use rand::{RngExt, seq::IteratorRandom};

use crate::{
    Instruction, InstructionContext, Operation, Program, ProgramContext, Variable,
    errors::ProgramValidationError,
};

struct Scope {
    /// Index of the instruction that opened the scope.
    begin: Option<usize>,
    id: usize,
    context: InstructionContext,
}

struct ScopedVariable {
    var: Variable,
    scope_id: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexedVariable {
    pub var: Variable,
    pub index: usize,
}

/// Builds a [`Program`], rejecting instructions whose inputs are of the wrong type or out of
/// scope. Every program that leaves [`ProgramBuilder::finalize`] is statically valid.
pub struct ProgramBuilder {
    context: ProgramContext,

    active_scopes: Vec<Scope>,
    active_scopes_set: HashSet<usize>,
    scope_counter: usize,

    variables: Vec<ScopedVariable>,
    instructions: Vec<Instruction>,
}

impl ProgramBuilder {
    #[must_use]
    pub fn new(context: ProgramContext) -> Self {
        let mut builder = Self {
            context,
            active_scopes: Vec::new(),
            active_scopes_set: HashSet::new(),
            scope_counter: 0,
            variables: Vec::new(),
            instructions: Vec::new(),
        };
        builder.enter_scope(None, InstructionContext::Global);
        builder
    }

    pub fn from_program(program: Program) -> Result<Self, ProgramValidationError> {
        Self::from_prefix(program.context, &program.instructions)
    }

    /// Rebuild the state a program is in after the given instructions.
    ///
    /// Scopes the prefix leaves open stay open, which is what lets a mutator ask what is in
    /// scope at the point it is rewriting.
    pub fn from_prefix(
        context: ProgramContext,
        instructions: &[Instruction],
    ) -> Result<Self, ProgramValidationError> {
        let mut builder = Self::new(context);
        for instruction in instructions {
            builder.append(instruction.clone())?;
        }
        Ok(builder)
    }

    #[must_use]
    pub fn context(&self) -> &ProgramContext {
        &self.context
    }

    #[must_use]
    pub fn variable_count(&self) -> usize {
        self.variables.len()
    }

    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }

    fn enter_scope(&mut self, begin: Option<usize>, context: InstructionContext) {
        self.scope_counter += 1;
        self.active_scopes.push(Scope {
            begin,
            id: self.scope_counter,
            context,
        });
        self.active_scopes_set.insert(self.scope_counter);
    }

    fn exit_scope(&mut self) -> Scope {
        let exited = self
            .active_scopes
            .pop()
            .expect("the global scope is never exited");
        self.active_scopes_set.remove(&exited.id);
        exited
    }

    fn current_scope(&self) -> &Scope {
        self.active_scopes
            .last()
            .expect("the global scope is never exited")
    }

    #[must_use]
    pub fn current_context(&self) -> &InstructionContext {
        &self.current_scope().context
    }

    /// Append an instruction, checking single assignment form and input types.
    pub fn append(
        &mut self,
        instruction: Instruction,
    ) -> Result<Vec<IndexedVariable>, ProgramValidationError> {
        if instruction.operation.num_inputs() != instruction.inputs.len() {
            return Err(ProgramValidationError::InvalidNumberOfInputs {
                is: instruction.inputs.len(),
                expected: instruction.operation.num_inputs(),
            });
        }

        let mut input_vars = Vec::with_capacity(instruction.inputs.len());
        for index in &instruction.inputs {
            let scoped = self
                .variables
                .get(*index)
                .ok_or(ProgramValidationError::VariableNotDefined(*index))?;
            if !self.active_scopes_set.contains(&scoped.scope_id) {
                return Err(ProgramValidationError::VariableNotDefined(*index));
            }
            input_vars.push(scoped.var.clone());
        }
        instruction.operation.check_input_types(&input_vars)?;

        match instruction.operation {
            Operation::LoadRole(i) if i >= self.context.num_roles => {
                return Err(ProgramValidationError::RoleNotFound(i));
            }
            Operation::LoadConnection(i) if i >= self.context.num_connections => {
                return Err(ProgramValidationError::ConnectionNotFound(i));
            }
            _ => {}
        }

        if instruction.operation.is_block_end() {
            let exited = self.exit_scope();
            let begin = &self.instructions[exited.begin.expect("a non global scope has a begin")];
            if !instruction.operation.is_matching_block_begin(&begin.operation) {
                return Err(ProgramValidationError::InvalidBlockEnd {
                    begin: begin.operation.clone(),
                    end: instruction.operation.clone(),
                });
            }
        }

        let first_new = self.variables.len();

        // Nop variables are placed in scope 0, which is never active, so they can never be
        // used as an input.
        let scope_id = match instruction.operation {
            Operation::Nop { .. } => 0,
            _ => self.current_scope().id,
        };
        self.variables.extend(
            instruction
                .operation
                .get_output_variables()
                .into_iter()
                .map(|var| ScopedVariable { var, scope_id }),
        );

        if instruction.operation.is_block_begin() {
            let context = instruction
                .entered_context_after_execution()
                .expect("a block beginning enters a context");
            self.enter_scope(Some(self.instructions.len()), context);
        }

        let inner_scope_id = match instruction.operation {
            Operation::Nop { .. } => 0,
            _ => self.scope_counter,
        };
        self.variables.extend(
            instruction
                .operation
                .get_inner_output_variables()
                .into_iter()
                .map(|var| ScopedVariable {
                    var,
                    scope_id: inner_scope_id,
                }),
        );

        self.instructions.push(instruction);

        Ok(self.variables[first_new..]
            .iter()
            .enumerate()
            .map(|(i, scoped)| IndexedVariable {
                var: scoped.var.clone(),
                index: first_new + i,
            })
            .collect())
    }

    /// Append an operation, resolving its inputs from the given variables.
    pub fn append_op(
        &mut self,
        operation: Operation,
        inputs: &[&IndexedVariable],
    ) -> Result<Vec<IndexedVariable>, ProgramValidationError> {
        let inputs = inputs.iter().map(|v| v.index).collect();
        self.append(Instruction::new(operation, inputs))
    }

    #[must_use]
    pub fn get_variable(&self, index: usize) -> Option<IndexedVariable> {
        let scoped = self.variables.get(index)?;
        self.active_scopes_set
            .contains(&scoped.scope_id)
            .then(|| IndexedVariable {
                var: scoped.var.clone(),
                index,
            })
    }

    /// A random in-scope variable of the given type.
    pub fn get_random_variable<R: RngExt>(
        &self,
        rng: &mut R,
        var: &Variable,
    ) -> Option<IndexedVariable> {
        (0..self.variables.len())
            .filter_map(|i| self.get_variable(i))
            .filter(|indexed| indexed.var == *var)
            .choose(rng)
    }

    /// The most recently defined in-scope variable of the given type.
    #[must_use]
    pub fn get_nearest_variable(&self, var: &Variable) -> Option<IndexedVariable> {
        (0..self.variables.len())
            .rev()
            .filter_map(|i| self.get_variable(i))
            .find(|indexed| indexed.var == *var)
    }

    pub fn finalize(self) -> Result<Program, ProgramValidationError> {
        if let Some(scope) = self.active_scopes.last()
            && let Some(begin) = scope.begin
        {
            return Err(ProgramValidationError::UnfinishedBlock(
                self.instructions[begin].operation.clone(),
            ));
        }
        Ok(Program {
            context: self.context,
            instructions: self.instructions,
        })
    }
}
