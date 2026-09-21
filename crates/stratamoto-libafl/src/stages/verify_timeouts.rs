use std::{cell::RefCell, fmt::Debug, marker::PhantomData, time::Duration};
use std::{collections::VecDeque, rc::Rc};

use libafl_bolts::Error;
use serde::{Deserialize, Serialize};

use libafl::{
    Evaluator, HasMetadata,
    executors::{Executor, HasObservers, HasTimeout, SetTimeout},
    observers::ObserversTuple,
    stages::{Restartable, Stage},
};

use crate::input::IrInput;

/// Stage that re-runs inputs deemed as timeouts with a multiple of the timeout to assert that they
/// are not false positives.
#[derive(Debug)]
pub struct VerifyTimeoutsStage<E, S> {
    multiple_of_timeout: Duration,
    original_timeout: Duration,
    capture_timeouts: Rc<RefCell<bool>>,
    phantom: PhantomData<(E, S)>,
}

impl<E, S> VerifyTimeoutsStage<E, S> {
    /// Create a `VerifyTimeoutsStage`
    pub fn new(
        capture_timeouts: Rc<RefCell<bool>>,
        configured_timeout: Duration,
        multiple: u32,
    ) -> Self {
        Self {
            capture_timeouts,
            multiple_of_timeout: configured_timeout * multiple,
            original_timeout: configured_timeout,
            phantom: PhantomData,
        }
    }
}

/// Timeouts that `VerifyTimeoutsStage` will read from
#[derive(Default, Serialize, Deserialize, Debug, Clone)]
pub struct TimeoutsToVerify {
    inputs: VecDeque<IrInput>,
}

libafl_bolts::impl_serdeany!(TimeoutsToVerify);

impl TimeoutsToVerify {
    /// Create a new `TimeoutsToVerify`
    #[must_use]
    pub fn new() -> Self {
        Self {
            inputs: VecDeque::new(),
        }
    }

    /// Add a `TimeoutsToVerify` to queue
    pub fn push(&mut self, input: IrInput) {
        self.inputs.push_back(input);
    }

    /// Pop a `TimeoutsToVerify` to queue
    pub fn pop(&mut self) -> Option<IrInput> {
        self.inputs.pop_front()
    }

    /// Count `TimeoutsToVerify` in queue
    #[must_use]
    pub fn count(&self) -> usize {
        self.inputs.len()
    }
}

impl<E, EM, S, Z> Stage<E, EM, S, Z> for VerifyTimeoutsStage<E, S>
where
    E::Observers: ObserversTuple<IrInput, S>,
    E: Executor<EM, IrInput, S, Z> + HasObservers + HasTimeout + SetTimeout,
    Z: Evaluator<E, EM, IrInput, S>,
    S: HasMetadata,
{
    fn perform(
        &mut self,
        fuzzer: &mut Z,
        executor: &mut E,
        state: &mut S,
        manager: &mut EM,
    ) -> Result<(), Error> {
        let mut timeouts = state.metadata_or_insert_with(TimeoutsToVerify::new).clone();
        if timeouts.count() == 0 {
            return Ok(());
        }
        tracing::info!("Verifying {} timeouts!", timeouts.count());
        executor.set_timeout(self.multiple_of_timeout);
        *self.capture_timeouts.borrow_mut() = false;
        while let Some(input) = timeouts.pop() {
            fuzzer.evaluate_input(state, executor, manager, &input)?;
        }
        executor.set_timeout(self.original_timeout);
        *self.capture_timeouts.borrow_mut() = true;
        let res = state.metadata_mut::<TimeoutsToVerify>().unwrap();
        *res = TimeoutsToVerify::new();
        Ok(())
    }
}

impl<E, S> Restartable<S> for VerifyTimeoutsStage<E, S> {
    fn should_restart(&mut self, _state: &mut S) -> Result<bool, Error> {
        Ok(true)
    }

    fn clear_progress(&mut self, _state: &mut S) -> Result<(), Error> {
        Ok(())
    }
}
