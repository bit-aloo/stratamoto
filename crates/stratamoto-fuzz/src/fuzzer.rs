use rand::RngExt;
use stratamoto_ir::{
    Program,
    generators::{
        Generator,
        raw_frame::RawFrameGenerator,
        setup_connection::{
            AdversarialSetupGenerator, ConnectionChoice, SetupConnectionGenerator, SetupViolation,
        },
    },
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
    observer::{NoObserver, Observer},
    target::{Behaviour, Outcome, Target},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct Stats {
    pub iterations: u64,
    pub skipped: u64,
    /// Mutations the builder rejected, which cost nothing but a clone.
    pub rejected: u64,
    pub corpus_additions: u64,
    /// Of the corpus additions, those the observer alone asked for: their behaviour was old
    /// and what they reached inside the target was not.
    pub coverage_additions: u64,
    /// Entries taken back from an earlier campaign's saved corpus.
    pub loaded: u64,
    pub failures: u64,
    /// Runs the harness could not give the program, which end the campaign.
    pub infrastructure: u64,
}

/// A crash the fuzzer found, reduced unless the target died with it.
pub struct Failure {
    pub program: Program,
    pub reason: String,
    /// Whether the target stopped serving. Such a program is reported as it was run: with the
    /// target down every candidate fails alike, so there is nothing for a minimizer to tell
    /// apart.
    pub killed_the_target: bool,
    /// The iteration it was found at; zero for a seed program, found before the campaign.
    pub iteration: u64,
    /// The corpus entry it was mutated from; none for a seed program, which was generated.
    pub parent: Option<Program>,
    /// Whether the reduced program failed the same way when run once more, which against a
    /// target that resets between runs means from a clean start.
    pub confirmed: bool,
    /// The record of the confirming run, or of the run that stopped the target, as the target
    /// encodes it.
    pub trace: Option<Vec<u8>>,
}

pub struct Fuzzer<T, R> {
    target: T,
    rng: R,
    observer: Box<dyn Observer>,
    corpus: Corpus,
    stats: Stats,
    /// Why the campaign cannot go on, once the harness has failed to run a program.
    stopped: Option<String>,
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
            observer: Box::new(NoObserver),
            corpus: Corpus::new(),
            stats: Stats::default(),
            stopped: None,
            num_roles,
            max_mutations: 4,
            max_instructions: 256,
        }
    }

    /// See inside the target as well as watch what it answers.
    #[must_use]
    pub fn with_observer(mut self, observer: Box<dyn Observer>) -> Self {
        self.observer = observer;
        self
    }

    /// Save every corpus entry to `dir` as it is admitted.
    pub fn with_corpus_dir(mut self, dir: impl Into<std::path::PathBuf>) -> std::io::Result<Self> {
        self.corpus = std::mem::take(&mut self.corpus).persisted_in(dir)?;
        Ok(self)
    }

    /// Take back what an earlier campaign saved in `dir`, running each program once so that
    /// its behaviour and coverage are this campaign's to compare against. Returns how many
    /// were kept; one that no longer behaves is a finding for the caller's own run to make.
    pub fn load_corpus(&mut self, dir: &std::path::Path) -> std::io::Result<usize> {
        let mut kept = 0;
        for program in Corpus::saved_in(dir)? {
            self.observer.reset();
            if let Outcome::Ok(behaviour) = self.target.run(&program) {
                self.observer.observe();
                self.corpus.restore(program, behaviour);
                kept += 1;
            }
        }
        self.stats.loaded += kept as u64;
        Ok(kept)
    }

    #[must_use]
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// How much of the target any run has reached, in the observer's unit.
    #[must_use]
    pub fn coverage(&self) -> usize {
        self.observer.seen()
    }

    #[must_use]
    pub fn observer_name(&self) -> &'static str {
        self.observer.name()
    }

    #[must_use]
    pub fn corpus(&self) -> &Corpus {
        &self.corpus
    }

    /// Why the campaign stopped, if the harness failed to run a program.
    #[must_use]
    pub fn stopped(&self) -> Option<&str> {
        self.stopped.as_deref()
    }

    /// End the campaign: the harness could not run a program, so no later run would mean
    /// anything.
    fn stop(&mut self, reason: String) {
        self.stats.infrastructure += 1;
        tracing::error!("ending the campaign: {reason}");
        self.stopped = Some(reason);
    }

    /// Fill the corpus with generated programs, one per role.
    ///
    /// A seed that fails is a finding like any other, and the first one worth knowing about:
    /// a target that rejects a plain generated setup would otherwise leave the corpus empty
    /// and the campaign silent. It is minimized and returned, and if it stopped the target
    /// the seeding ends there.
    #[must_use]
    pub fn seed(&mut self, context: stratamoto_ir::ProgramContext) -> Vec<Failure> {
        let mut failures = Vec::new();

        for role in 0..self.num_roles {
            let mut builder = stratamoto_ir::ProgramBuilder::new(context.clone());
            let generator = SetupConnectionGenerator::new(role);
            if generator.generate(&mut builder, &mut self.rng).is_err() {
                continue;
            }
            let Ok(program) = builder.finalize() else {
                continue;
            };

            self.observer.reset();
            match self.target.run(&program) {
                Outcome::Ok(behaviour) => {
                    self.admit(program, behaviour, None);
                }
                Outcome::Skip => self.stats.skipped += 1,
                Outcome::Fail(reason) => {
                    let failure = self.failure(program, reason, None);
                    let killed_the_target = failure.killed_the_target;
                    failures.push(failure);
                    if killed_the_target {
                        tracing::warn!("the target stopped serving while seeding");
                        break;
                    }
                }
                Outcome::Infrastructure(reason) => {
                    self.stop(reason);
                    break;
                }
            }
            if self.stopped.is_some() {
                break;
            }
        }

        failures
    }

    /// Run one iteration. Returns a failure if the target violated a property.
    pub fn run_one(&mut self) -> Option<Failure> {
        self.stats.iterations += 1;

        let picked = self.corpus.pick(&mut self.rng)?;
        let parent = self.corpus.program(picked).clone();
        let Some(mutated) = self.mutate(parent.clone()) else {
            self.stats.rejected += 1;
            return None;
        };

        self.observer.reset();
        match self.target.run(&mutated) {
            Outcome::Ok(behaviour) => {
                self.admit(mutated, behaviour, Some(picked));
                None
            }
            Outcome::Skip => {
                self.stats.skipped += 1;
                None
            }
            Outcome::Fail(reason) => Some(self.failure(mutated, reason, Some(parent))),
            Outcome::Infrastructure(reason) => {
                self.stop(reason);
                None
            }
        }
    }

    /// Keep a program that behaved if it is new to the corpus by either measure: what it drew
    /// out of the target, or what it reached inside it; and credit the entry it came from.
    fn admit(&mut self, program: Program, behaviour: Behaviour, parent: Option<usize>) {
        let reached_new = self.observer.observe();
        let kept = if reached_new {
            self.corpus.keep(program, behaviour);
            self.stats.coverage_additions += 1;
            true
        } else {
            self.corpus.add(program, behaviour)
        };
        if kept {
            self.stats.corpus_additions += 1;
            if let Some(parent) = parent {
                self.corpus.credit(parent);
            }
        }
    }

    /// Account for a failing program and reduce it, unless the target died with it.
    fn failure(&mut self, program: Program, reason: String, parent: Option<Program>) -> Failure {
        self.stats.failures += 1;
        let iteration = self.stats.iterations;
        if !self.target.is_alive() {
            return Failure {
                trace: self.target.trace(),
                program,
                reason,
                killed_the_target: true,
                iteration,
                parent,
                confirmed: false,
            };
        }
        let program = self.minimize(program, &reason);
        // Once more, so that what is reported is known to reproduce, and so that the trace
        // saved with it is the trace of the program saved with it.
        let confirmed =
            matches!(self.target.run(&program), Outcome::Fail(other) if other == reason);
        Failure {
            trace: confirmed.then(|| self.target.trace()).flatten(),
            program,
            reason,
            killed_the_target: false,
            iteration,
            parent,
            confirmed,
        }
    }

    /// Run up to `iterations` iterations, fewer if the target dies or the harness fails.
    pub fn run(&mut self, iterations: u64) -> Vec<Failure> {
        let mut failures = Vec::new();
        for _ in 0..iterations {
            if self.stopped.is_some() {
                break;
            }
            if let Some(failure) = self.run_one() {
                let killed_the_target = failure.killed_the_target;
                failures.push(failure);
                if killed_the_target {
                    tracing::warn!("the target stopped serving; ending the campaign");
                    break;
                }
            }
        }
        failures
    }

    fn mutate(&mut self, mut program: Program) -> Option<Program> {
        let rounds = self.rng.random_range(1..=self.max_mutations);
        let mut applied = false;

        for _ in 0..rounds {
            let role = self.rng.random_range(0..self.num_roles);
            let result = match self.rng.random_range(0..7u8) {
                0 => InputMutator.mutate(&mut program, &mut self.rng),
                1 => OperationMutator.mutate(&mut program, &mut self.rng),
                2 => ConcatMutator.mutate(&mut program, &mut self.rng),
                3 => GeneratorMutator::new(RawFrameGenerator { role })
                    .mutate(&mut program, &mut self.rng),
                // A valid setup on a fresh connection is what reaches deep states, and one on
                // a connection the program already has is a named violation in its own right.
                4 => GeneratorMutator::new(SetupConnectionGenerator::new(role))
                    .mutate(&mut program, &mut self.rng),
                5 => GeneratorMutator::new(SetupConnectionGenerator {
                    role,
                    protocol: None,
                    connection: ConnectionChoice::Reuse,
                })
                .mutate(&mut program, &mut self.rng),
                _ => GeneratorMutator::new(AdversarialSetupGenerator {
                    role,
                    protocol: None,
                    violation: SetupViolation::any(&mut self.rng),
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
            if self.stopped.is_some() {
                break;
            }
            match self.target.run(&candidate) {
                // Only the same failure counts; a different one would be a different bug.
                Outcome::Fail(other) if other == reason => minimizer.success(),
                // The harness, not the candidate, failed: what is left is reported as it is
                // rather than shrunk against a target that cannot be trusted to run it.
                Outcome::Infrastructure(why) => self.stop(why),
                _ => minimizer.failure(),
            }
        }
        minimizer.current().clone()
    }
}
