use crate::{Program, minimizers::Minimizer};

/// Removes a whole block, from its beginning to its matching end.
///
/// A block and its contents only make sense together, so nopping cannot take one out: the
/// delimiters are not noppable and removing either alone leaves the scopes unbalanced.
pub struct BlockMinimizer {
    program: Program,
    candidate: Option<Program>,
    next: usize,
}

impl Minimizer for BlockMinimizer {
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
            self.next = 0;
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

impl Iterator for BlockMinimizer {
    type Item = Program;

    fn next(&mut self) -> Option<Program> {
        while self.next < self.program.instructions.len() {
            let begin = self.next;
            if !self.program.instructions[begin].operation.is_block_begin() {
                self.next += 1;
                continue;
            }

            let Some(end) = matching_end(&self.program, begin) else {
                self.next += 1;
                continue;
            };

            let mut remove = vec![false; self.program.instructions.len()];
            for flag in remove.iter_mut().take(end + 1).skip(begin) {
                *flag = true;
            }

            let candidate = self.program.without(&remove);
            if candidate.is_statically_valid() {
                self.candidate = Some(candidate.clone());
                return Some(candidate);
            }
            self.next += 1;
        }
        None
    }
}

fn matching_end(program: &Program, begin: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, instruction) in program.instructions.iter().enumerate().skip(begin) {
        if instruction.operation.is_block_begin() {
            depth += 1;
        }
        if instruction.operation.is_block_end() {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}
