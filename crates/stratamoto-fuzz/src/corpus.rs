use std::collections::HashSet;

use rand::{RngExt, seq::IndexedRandom};
use stratamoto_ir::Program;

/// The programs the fuzzer mutates, one entry per behaviour seen so far.
///
/// Without coverage from inside the roles the only feedback available is what a run did
/// through the protocol, so a program earns a place by producing a signature no other
/// program has.
#[derive(Default)]
pub struct Corpus {
    programs: Vec<Program>,
    signatures: HashSet<u64>,
}

impl Corpus {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.programs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }

    /// Keep the program if its signature has not been seen. Returns whether it was kept.
    pub fn add(&mut self, program: Program, signature: u64) -> bool {
        if !self.signatures.insert(signature) {
            return false;
        }
        self.programs.push(program);
        true
    }

    pub fn pick<R: RngExt>(&self, rng: &mut R) -> Option<&Program> {
        self.programs.choose(rng)
    }

    #[must_use]
    pub fn programs(&self) -> &[Program] {
        &self.programs
    }
}
