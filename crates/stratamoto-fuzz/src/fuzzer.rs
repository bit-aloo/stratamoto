use rand::RngExt;
use stratamoto_ir::{
    Program,
    generators::{Generator, setup_connection::SetupConnectionGenerator},
    minimizers::{
        Minimizer, block::BlockMinimizer, cutting::CuttingMinimizer, nopping::NoppingMinimizer,
    },
    mutators::{
        Mutator, concat::ConcatMutator, generate::GeneratorMutator, input::InputMutator,
        operation::OperationMutator,
    },
};

use crate::{
    corpus::Corpus,
    target::{Outcome, Target},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub iterations: u64,
    pub skipped: u64,
    /// Mutations the builder rejected, which cost nothing but a clone.
    pub rejected: u64,
    pub corpus_additions: u64,
    pub failures: u64,
}

/// A crash the fuzzer found, already reduced.
pub struct Failure {
    pub program: Program,
    pub reason: String,
}

pub struct Fuzzer<T, R> {
    target: T,
    rng: R,
    corpus: Corpus,
    stats: Stats,
    /// Roles a generated fragment may connect to.
    num_roles: usize,
    /// How many mutations to stack on one program before running it.
    max_mutations: u32,
    /// Programs past this length are discarded. Concatenation doubles a program, so without
    /// a cap the corpus grows until a single run dominates the budget.
    max_instructions: usize,
}

impl<T: Target, R: RngExt> Fuzzer<T, R> {
    pub fn new(target: T, rng: R, num_roles: usize) -> Self {
        Self {
            target,
            rng,
            corpus: Corpus::new(),
            stats: Stats::default(),
            num_roles,
            max_mutations: 4,
            max_instructions: 256,
        }
    }

    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    #[must_use]
    pub fn corpus(&self) -> &Corpus {
        &self.corpus
    }

    /// Fill the corpus with generated programs, one per role.
    pub fn seed(&mut self, context: stratamoto_ir::ProgramContext) {
        for role in 0..self.num_roles {
            let mut builder = stratamoto_ir::ProgramBuilder::new(context.clone());
            let generator = SetupConnectionGenerator { role };
            if generator.generate(&mut builder, &mut self.rng).is_err() {
                continue;
            }
            let Ok(program) = builder.finalize() else {
                continue;
            };
            match self.target.run(&program) {
                Outcome::Ok { signature } => {
                    if self.corpus.add(program, signature) {
                        self.stats.corpus_additions += 1;
                    }
                }
                // A seed that already fails is reported by the caller's own run.
                _ => {}
            }
        }
    }

    /// Run one iteration. Returns a failure if the target violated a property.
    pub fn run_one(&mut self) -> Option<Failure> {
        self.stats.iterations += 1;

        let Some(program) = self.corpus.pick(&mut self.rng).cloned() else {
            return None;
        };
        let Some(mutated) = self.mutate(program) else {
            self.stats.rejected += 1;
            return None;
        };

        match self.target.run(&mutated) {
            Outcome::Ok { signature } => {
                if self.corpus.add(mutated, signature) {
                    self.stats.corpus_additions += 1;
                }
                None
            }
            Outcome::Skip => {
                self.stats.skipped += 1;
                None
            }
            Outcome::Fail(reason) => {
                self.stats.failures += 1;
                let program = self.minimize(mutated, &reason);
                Some(Failure { program, reason })
            }
        }
    }

    pub fn run(&mut self, iterations: u64) -> Vec<Failure> {
        let mut failures = Vec::new();
        for _ in 0..iterations {
            if let Some(failure) = self.run_one() {
                failures.push(failure);
            }
        }
        failures
    }

    fn mutate(&mut self, mut program: Program) -> Option<Program> {
        let rounds = self.rng.random_range(1..=self.max_mutations);
        let mut applied = false;

        for _ in 0..rounds {
            let result = match self.rng.random_range(0..4u8) {
                0 => InputMutator.mutate(&mut program, &mut self.rng),
                1 => OperationMutator.mutate(&mut program, &mut self.rng),
                2 => ConcatMutator.mutate(&mut program, &mut self.rng),
                _ => GeneratorMutator::new(SetupConnectionGenerator {
                    role: self.rng.random_range(0..self.num_roles),
                })
                .mutate(&mut program, &mut self.rng),
            };
            applied |= result.is_ok();
        }

        if !applied || program.instructions.len() > self.max_instructions {
            return None;
        }
        // A mutator may leave a program the builder would not accept, which is cheaper to
        // discard here than to run.
        program.is_statically_valid().then_some(program)
    }

    /// Reduce a failing program to the smallest one that still fails the same way.
    ///
    /// Each pass runs forward once, so an instruction can only go after whatever consumed it
    /// has already gone, and an empty block only after its contents have. Repeating the
    /// pipeline until it stops shrinking is what lets a chain of dead instructions unwind.
    fn minimize(&mut self, program: Program, reason: &str) -> Program {
        let mut program = program;

        loop {
            let before = program.instructions.len();

            program = self.minimize_with::<CuttingMinimizer>(program, reason);
            program = self.minimize_with::<BlockMinimizer>(program, reason);
            program = self.minimize_with::<NoppingMinimizer>(program, reason);
            program.remove_nops();

            if program.instructions.len() >= before {
                return program;
            }
        }
    }

    fn minimize_with<M: Minimizer>(&mut self, program: Program, reason: &str) -> Program {
        let mut minimizer = M::new(program);
        while let Some(candidate) = minimizer.next() {
            match self.target.run(&candidate) {
                // Only the same failure counts; a different one would be a different bug.
                Outcome::Fail(other) if other == reason => minimizer.success(),
                _ => minimizer.failure(),
            }
        }
        minimizer.current().clone()
    }
}
