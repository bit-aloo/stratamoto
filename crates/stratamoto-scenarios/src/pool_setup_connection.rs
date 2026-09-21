use stratamoto::{
    backend::{ExecutionBackend, SnapshotId},
    error::{Error, Result},
    runner::{self, Execution},
    scenario::{Scenario, ScenarioResult},
    transport::Deployment,
};
pub use stratamoto_targets::backend::Reset;
use stratamoto_targets::{backend::RestartBackend, pool::PoolDeployment};

use crate::setup_connection::{Run, TestCase};

/// The setup connection scenario against sv2-apps' pool instead of the mock roles.
///
/// The pool sits behind the restart backend, which replaces it between test cases per the
/// [`Reset`] policy and checks with a canary that each case starts from the root, so no case
/// sees what earlier ones left behind.
pub struct PoolSetupConnectionScenario {
    backend: RestartBackend,
}

impl PoolSetupConnectionScenario {
    /// Bring the pool up and take the root every test case starts from.
    pub fn start() -> Result<Self> {
        Self::start_with(Reset::Pool)
    }

    pub fn start_with(reset: Reset) -> Result<Self> {
        // The canary costs a run and a restart, so it is thinned out to one input in ten.
        let mut backend = RestartBackend::new(reset).with_canary_every(10);
        backend
            .prepare_scenario()
            .map_err(|e| Error::Target(e.to_string()))?;
        backend
            .take_root_snapshot()
            .map_err(|e| Error::Target(e.to_string()))?;
        Ok(Self { backend })
    }

    /// The root every test case starts from.
    #[must_use]
    pub fn snapshot(&self) -> Option<&SnapshotId> {
        self.backend.snapshot()
    }

    #[must_use]
    pub fn num_roles(&self) -> usize {
        self.backend
            .deployment()
            .map_or(0, PoolDeployment::num_roles)
    }

    /// Whether the scenario can still be run against.
    ///
    /// A scenario that replaces the pool between runs can, whatever the last run did to it: a
    /// pool that stopped serving is a finding about that run, recorded by its oracle, and not a
    /// reason the next run cannot happen. Only one that never resets is tied to the pool's fate.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.backend.is_alive()
    }

    pub fn execute(&mut self, testcase: &TestCase) -> Run {
        // A program written for more roles than the pool deployment has would open connections
        // to nothing, which is a mismatch with the input rather than a finding about the pool.
        if testcase.program.context.num_roles > self.num_roles() {
            let deployment = self.backend.deployment().expect("prepared at start");
            return Run::of(
                deployment,
                &testcase.program,
                Execution::default(),
                |_, _, _| ScenarioResult::Skip,
            );
        }

        let deployment = match self.backend.next_input() {
            Ok(deployment) => deployment,
            Err(e) => {
                let reason = format!("the pool could not be brought to the root: {e}");
                let deployment = self.backend.deployment().expect("prepared at start");
                return Run::of(
                    deployment,
                    &testcase.program,
                    Execution::default(),
                    |_, _, _| ScenarioResult::Infrastructure(reason),
                );
            }
        };

        let execution = runner::run(deployment, &testcase.program);
        let run = Run::of(
            deployment,
            &testcase.program,
            execution,
            crate::setup_connection::evaluate,
        );
        if let Err(e) = self.backend.report_and_reset(&run.digest.verdict) {
            log::warn!("the pool could not be reset after a run: {e}");
        }
        run
    }
}

impl Scenario<TestCase> for PoolSetupConnectionScenario {
    /// A program's seed has no effect here: the pool runs on its own runtime, not on the
    /// simulator.
    fn new() -> Result<Self> {
        Self::start()
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        self.execute(&testcase).result
    }
}
