use stratamoto::{
    error::{Error, Result},
    runner::{self, Execution},
    scenario::{Scenario, ScenarioResult},
    transport::Deployment,
};
use stratamoto_targets::pool::PoolDeployment;

use crate::setup_connection::{Run, TestCase};

/// The setup connection scenario against sv2-apps' pool.
///
/// One pool serves the whole life of the scenario. What keeps test cases apart is that a
/// scenario process runs one of them: locally, one input per process; under a snapshotting
/// fuzzer, one input per restore of the snapshot taken after the pool came up.
pub struct PoolSetupConnectionScenario {
    deployment: PoolDeployment,
}

impl PoolSetupConnectionScenario {
    /// Bring the pool up, with the node and `sv2-tp` behind it.
    pub fn start() -> Result<Self> {
        let deployment = PoolDeployment::start().map_err(|e| Error::Target(e.to_string()))?;
        Ok(Self { deployment })
    }

    #[must_use]
    pub fn deployment(&self) -> &PoolDeployment {
        &self.deployment
    }

    #[must_use]
    pub fn num_roles(&self) -> usize {
        self.deployment.num_roles()
    }

    /// Whether the pool is still serving.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.deployment.is_alive()
    }

    pub fn execute(&mut self, testcase: &TestCase) -> Run {
        // A program written for more roles than the pool deployment has would open connections
        // to nothing, which is a mismatch with the input rather than a finding about the pool.
        if testcase.program.context.num_roles > self.num_roles() {
            return Run::of(
                &self.deployment,
                &testcase.program,
                Execution::default(),
                |_, _, _| ScenarioResult::Skip,
            );
        }

        let execution = runner::run(&self.deployment, &testcase.program);
        Run::of(
            &self.deployment,
            &testcase.program,
            execution,
            crate::setup_connection::evaluate,
        )
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
