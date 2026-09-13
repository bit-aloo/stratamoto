use crate::{Program, minimizers::Minimizer};

/// Replaces one instruction at a time with a nop.
///
/// Nopping keeps every later input index valid, so the program stays runnable no matter which
/// instruction is removed. The nops are dropped once minimization finishes.
pub struct NoppingMinimizer {
    program: Program,
    candidate: Option<Program>,
    next: usize,
}

impl Minimizer for NoppingMinimizer {
    fn new(program: Program) -> Self {
        Self {
            program,
            candidate: None,
            next: 0,
        }
    }

    fn success(&mut self) {
        if let Some(candidate) = self.candidate.take() {
            self.program = candidate;
        }
    }

    fn failure(&mut self) {
        self.candidate = None;
        self.next += 1;
    }

    fn current(&self) -> &Program {
        &self.program
    }
}

impl Iterator for NoppingMinimizer {
    type Item = Program;

    fn next(&mut self) -> Option<Program> {
        while self.next < self.program.instructions.len() {
            let instruction = &self.program.instructions[self.next];
            if !instruction.is_noppable() {
                self.next += 1;
                continue;
            }
            if matches!(instruction.operation, crate::Operation::Nop { .. }) {
                self.next += 1;
                continue;
            }

            let mut candidate = self.program.clone();
            candidate.instructions[self.next].nop();
            if !candidate.is_statically_valid() {
                self.next += 1;
                continue;
            }

            self.candidate = Some(candidate.clone());
            return Some(candidate);
        }
        None
    }
}
