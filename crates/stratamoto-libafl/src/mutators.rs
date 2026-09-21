//! The IR's mutators and generators as LibAFL mutators.
//!
//! A mutation is kept only if the program it made is statically valid and within the
//! instruction limit; the IR's mutators may leave a program invalid and expect the caller to
//! discard it.

use std::borrow::Cow;

use libafl::{
    Error,
    corpus::{Corpus, CorpusId},
    mutators::{MutationResult, Mutator},
    random_corpus_id,
    state::{HasCorpus, HasRand},
};
use libafl_bolts::{Named, rands::Rand};
use rand::{RngExt, rngs::SmallRng};
use stratamoto_ir::{
    Program, ProgramBuilder,
    generators::{
        Generator,
        raw_frame::RawFrameGenerator,
        setup_connection::{AdversarialSetupGenerator, SetupConnectionGenerator, SetupViolation},
    },
    mutators::{Mutator as IrMutation, Splicer},
};

use crate::input::IrInput;

/// Instruction limit for mutated programs.
const MAX_INSTRUCTIONS: usize = 4096;

fn acceptable(program: &Program) -> bool {
    program.instructions.len() <= MAX_INSTRUCTIONS && program.is_statically_valid()
}

pub struct IrMutator<M> {
    mutator: M,
    rng: SmallRng,
    name: Cow<'static, str>,
}

impl<M: IrMutation<SmallRng>> IrMutator<M> {
    pub fn new(mutator: M, rng: SmallRng) -> Self {
        let name = Cow::from(mutator.name());
        Self { mutator, rng, name }
    }
}

impl<S, M> Mutator<IrInput, S> for IrMutator<M>
where
    M: IrMutation<SmallRng>,
{
    fn mutate(&mut self, _state: &mut S, input: &mut IrInput) -> Result<MutationResult, Error> {
        let mut candidate = input.ir().clone();
        if self.mutator.mutate(&mut candidate, &mut self.rng).is_err() || !acceptable(&candidate) {
            return Ok(MutationResult::Skipped);
        }
        *input.ir_mut() = candidate;
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

impl<M> Named for IrMutator<M> {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

pub struct IrSpliceMutator<M> {
    mutator: M,
    rng: SmallRng,
    name: Cow<'static, str>,
}

impl<M: IrMutation<SmallRng> + Splicer<SmallRng>> IrSpliceMutator<M> {
    pub fn new(mutator: M, rng: SmallRng) -> Self {
        let name = Cow::from(mutator.name());
        Self { mutator, rng, name }
    }
}

impl<S, M> Mutator<IrInput, S> for IrSpliceMutator<M>
where
    S: HasRand + HasCorpus<IrInput>,
    M: IrMutation<SmallRng> + Splicer<SmallRng>,
{
    fn mutate(&mut self, state: &mut S, input: &mut IrInput) -> Result<MutationResult, Error> {
        let id = random_corpus_id!(state.corpus(), state.rand_mut());

        // Splicing an input with itself is concatenation, which the concat mutator does.
        if let Some(cur) = state.corpus().current()
            && id == *cur
        {
            return Ok(MutationResult::Skipped);
        }

        let mut other_testcase = state.corpus().get_from_all(id)?.borrow_mut();
        if other_testcase.scheduled_count() == 0 {
            // Don't splice with inputs that have not been minimized.
            return Ok(MutationResult::Skipped);
        }
        let other = other_testcase.load_input(state.corpus())?;

        let mut candidate = input.ir().clone();
        if self
            .mutator
            .splice(&mut candidate, other.ir(), &mut self.rng)
            .is_err()
            || !acceptable(&candidate)
        {
            return Ok(MutationResult::Skipped);
        }
        *input.ir_mut() = candidate;
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

impl<M> Named for IrSpliceMutator<M> {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

/// Inserts a generated fragment at a random point of the program that is in the global
/// context, rather than only at its end.
pub struct IrGenerator<G> {
    generator: G,
    rng: SmallRng,
    name: Cow<'static, str>,
}

impl<G: Generator<SmallRng>> IrGenerator<G> {
    pub fn new(generator: G, rng: SmallRng, name: impl Into<Cow<'static, str>>) -> Self {
        Self {
            generator,
            rng,
            name: name.into(),
        }
    }
}

impl<S, G> Mutator<IrInput, S> for IrGenerator<G>
where
    G: Generator<SmallRng>,
{
    fn mutate(&mut self, _state: &mut S, input: &mut IrInput) -> Result<MutationResult, Error> {
        let program = input.ir();

        // Only a point outside any block will take a fragment that opens connections and
        // sends on them.
        let mut depth = 0usize;
        let mut points = vec![0usize];
        for (index, instruction) in program.instructions.iter().enumerate() {
            if instruction.operation.is_block_begin() {
                depth += 1;
            } else if instruction.operation.is_block_end() {
                depth = depth.saturating_sub(1);
            }
            if depth == 0 {
                points.push(index + 1);
            }
        }
        let index = points[self.rng.random_range(0..points.len())];

        let Ok(mut builder) =
            ProgramBuilder::from_prefix(program.context.clone(), &program.instructions[..index])
        else {
            return Ok(MutationResult::Skipped);
        };
        let before = builder.variable_count();
        if self
            .generator
            .generate(&mut builder, &mut self.rng)
            .is_err()
        {
            return Ok(MutationResult::Skipped);
        }
        let added = builder.variable_count() - before;

        // The rest of the program refers to variables numbered before the fragment was
        // inserted; those at or past the insertion point move up by what the fragment made.
        let mut rest = program.instructions[index..].to_vec();
        for instruction in &mut rest {
            for input in &mut instruction.inputs {
                if *input >= before {
                    *input += added;
                }
            }
        }
        let mut instructions = builder.instructions().to_vec();
        instructions.extend(rest);
        let candidate = Program::unchecked_new(program.context.clone(), instructions);
        if !acceptable(&candidate) {
            return Ok(MutationResult::Skipped);
        }
        *input.ir_mut() = candidate;
        Ok(MutationResult::Mutated)
    }

    fn post_exec(&mut self, _state: &mut S, _new_corpus_id: Option<CorpusId>) -> Result<(), Error> {
        Ok(())
    }
}

impl<G> Named for IrGenerator<G> {
    fn name(&self) -> &Cow<'static, str> {
        &self.name
    }
}

/// A conforming setup to a role drawn at random from the deployment.
pub struct AnyRoleSetup {
    pub num_roles: usize,
}

impl<R: RngExt> Generator<R> for AnyRoleSetup {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), stratamoto_ir::ProgramValidationError> {
        let role = rng.random_range(0..self.num_roles.max(1));
        SetupConnectionGenerator::new(role).generate(builder, rng)
    }
}

/// A setup that violates one constraint, drawn at random, to a role drawn at random.
pub struct AnyRoleAdversarialSetup {
    pub num_roles: usize,
}

impl<R: RngExt> Generator<R> for AnyRoleAdversarialSetup {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), stratamoto_ir::ProgramValidationError> {
        let generator = AdversarialSetupGenerator {
            role: rng.random_range(0..self.num_roles.max(1)),
            protocol: None,
            violation: SetupViolation::any(rng),
        };
        generator.generate(builder, rng)
    }
}

/// A raw frame on some connection, opened to a role drawn at random if there is none.
pub struct AnyRoleRawFrame {
    pub num_roles: usize,
}

impl<R: RngExt> Generator<R> for AnyRoleRawFrame {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), stratamoto_ir::ProgramValidationError> {
        let role = rng.random_range(0..self.num_roles.max(1));
        RawFrameGenerator { role }.generate(builder, rng)
    }
}
