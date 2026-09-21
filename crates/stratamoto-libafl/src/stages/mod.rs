pub mod stability_check;
pub use stability_check::*;

pub mod verify_timeouts;
pub use verify_timeouts::*;

use std::{borrow::Borrow, cell::RefCell, marker::PhantomData};

use libafl::{
    Evaluator, ExecutesInput, HasMetadata,
    events::EventFirer,
    executors::{Executor, ExitKind, HasObservers},
    feedbacks::MapNoveltiesMetadata,
    inputs::Input,
    observers::{CanTrack, MapObserver, ObserversTuple},
    stages::{Restartable, Stage},
    state::{HasCorpus, HasCurrentTestcase},
};
use libafl_bolts::tuples::Handle;
use stratamoto_ir::minimizers::Minimizer;

use crate::input::IrInput;

/// Reduces a corpus entry with one of the IR's minimizers, keeping a candidate when it still
/// covers what the entry was kept for, or, when minimizing a crash, still fails.
pub struct IrMinimizerStage<'a, M, T, O> {
    trace_handle: Handle<T>,
    consecutive_failures: usize,
    max_consecutive_failures: usize,
    minimizing_crash: bool,
    keep_minimizing: &'a RefCell<u64>,
    _phantom: PhantomData<(M, O)>,
}

impl<'a, M, T, O> IrMinimizerStage<'a, M, T, O>
where
    O: MapObserver,
    T: AsRef<O> + CanTrack,
    M: Minimizer,
{
    pub fn new(
        trace_handle: Handle<T>,
        max_consecutive_failures: usize,
        minimizing_crash: bool,
        keep_minimizing: &'a RefCell<u64>,
    ) -> Self {
        Self {
            trace_handle,
            consecutive_failures: 0,
            max_consecutive_failures,
            minimizing_crash,
            keep_minimizing,
            _phantom: PhantomData,
        }
    }
}

impl<M, T, O, S> Restartable<S> for IrMinimizerStage<'_, M, T, O> {
    fn should_restart(&mut self, _state: &mut S) -> Result<bool, libafl::Error> {
        Ok(true)
    }

    fn clear_progress(&mut self, _state: &mut S) -> Result<(), libafl::Error> {
        Ok(())
    }
}

impl<M, E, EM, S, Z, OT, T, O> Stage<E, EM, S, Z> for IrMinimizerStage<'_, M, T, O>
where
    M: Minimizer,
    S: HasCorpus<IrInput> + HasCurrentTestcase<IrInput> + HasMetadata,
    E: Executor<EM, IrInput, S, Z> + HasObservers<Observers = OT>,
    EM: EventFirer<IrInput, S>,
    Z: Evaluator<E, EM, IrInput, S> + ExecutesInput<E, EM, IrInput, S>,
    OT: ObserversTuple<IrInput, S>,
    O: MapObserver,
    T: CanTrack + AsRef<O>,
{
    fn perform(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        state: &mut S,
        manager: &mut EM,
    ) -> Result<(), libafl::Error> {
        if state.current_testcase()?.scheduled_count() > 0 {
            // Already minimized.
            return Ok(());
        }

        let novelties = state
            .current_testcase()?
            .borrow()
            .metadata::<MapNoveltiesMetadata>()
            .map(|m| m.list.clone())
            .unwrap_or_default();

        let mut success = false;
        let mut current_ir = state.current_input_cloned()?;

        let name = std::any::type_name::<M>();
        tracing::info!(
            "{name} reducing ir: {} instrs",
            current_ir.ir().instructions.len()
        );
        let mut minimizer = M::new(current_ir.ir().clone());
        while let Some(prog) = minimizer.next() {
            if self.consecutive_failures > self.max_consecutive_failures {
                break;
            }

            if !prog.is_statically_valid() {
                tracing::debug!("{name} failure (not statically valid)");
                minimizer.failure();
                self.consecutive_failures += 1;
                continue;
            }

            let attempt = IrInput::new(prog);
            let Ok(exit_kind) = fuzzer.execute_input(state, executor, manager, &attempt) else {
                continue;
            };

            let number_of_retained_novelties = executor.observers()[&self.trace_handle]
                .as_ref()
                .how_many_set(&novelties);
            if (self.minimizing_crash && exit_kind != ExitKind::Ok)
                || (!self.minimizing_crash && number_of_retained_novelties == novelties.len())
            {
                // The candidate still has all the same novelties.
                success = true;
                current_ir = attempt;
                minimizer.success();
                tracing::debug!("{name} success");
                self.consecutive_failures = 0;
            } else {
                minimizer.failure();
                tracing::debug!("{name} failure");
                self.consecutive_failures += 1;
            }
        }

        tracing::info!("{name} done reducing");

        if success {
            *self.keep_minimizing.borrow_mut() += 1;
            current_ir.ir_mut().remove_nops();

            tracing::info!(
                "{name} reduced ir to {} instructions",
                current_ir.ir().instructions.len()
            );

            let mut testcase = state.current_testcase_mut()?;
            testcase.set_input(current_ir);
            let filepath = testcase.file_path().as_ref().unwrap().clone();
            tracing::info!("{name} reduced ir written to: {}", filepath.display());
            let _ = testcase.input().as_ref().unwrap().to_file(filepath);
        }

        Ok(())
    }
}
