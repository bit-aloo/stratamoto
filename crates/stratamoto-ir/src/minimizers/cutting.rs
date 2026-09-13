use crate::{Program, minimizers::Minimizer};

/// Removes a run of instructions from the end, halving the run each time it is rejected.
///
/// Cutting is what takes a long program down quickly; nopping then removes what is left one
/// instruction at a time.
pub struct CuttingMinimizer {
    program: Program,
    candidate: Option<Program>,
    length: usize,
}

impl Minimizer for CuttingMinimizer {
    fn new(program: Program) -> Self {
        let length = program.instructions.len() / 2;
        Self {
            program,
            candidate: None,
            length,
        }
    }

    fn success(&mut self) {
        if let Some(candidate) = self.candidate.take() {
            self.program = candidate;
            self.length = self.length.min(self.program.instructions.len() / 2);
        }
    }

    fn failure(&mut self) {
        self.candidate = None;
        self.length /= 2;
    }

    fn current(&self) -> &Program {
        &self.program
    }
}

impl Iterator for CuttingMinimizer {
    type Item = Program;

    fn next(&mut self) -> Option<Program> {
        while self.length > 0 {
            let total = self.program.instructions.len();
            let mut remove = vec![false; total];
            for flag in remove.iter_mut().skip(total - self.length) {
                *flag = true;
            }

            let candidate = self.program.without(&remove);
            if candidate.is_statically_valid() {
                self.candidate = Some(candidate.clone());
                return Some(candidate);
            }
            self.length /= 2;
        }
        None
    }
}
