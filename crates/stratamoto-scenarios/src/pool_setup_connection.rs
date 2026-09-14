use stratamoto::{
    error::{Error, Result},
    oracle::{Oracle, OracleResult, SetupConnectionOracle},
    runner::{self, Execution},
    scenario::{Scenario, ScenarioResult},
    transport::Deployment,
};
use stratamoto_targets::pool::PoolDeployment;

use crate::setup_connection::TestCase;

/// The setup connection scenario against sv2-apps' pool instead of the mock roles.
///
/// The pool is started once and every test case opens fresh connections to it, so a case can
/// be affected by whatever earlier ones left behind in the pool.
pub struct PoolSetupConnectionScenario {
    deployment: PoolDeployment,
}

impl PoolSetupConnectionScenario {
    pub fn start() -> Result<Self> {
        let deployment = PoolDeployment::start().map_err(|e| Error::Target(e.to_string()))?;
        Ok(Self { deployment })
    }

    #[must_use]
    pub fn num_roles(&self) -> usize {
        self.deployment.num_roles()
    }

    pub fn execute(&self, testcase: &TestCase) -> (Execution, ScenarioResult) {
        // A program written for more roles than the pool deployment has would open connections
        // to nothing, which is a mismatch with the input rather than a finding about the pool.
        if testcase.program.context.num_roles > self.num_roles() {
            return (Execution::default(), ScenarioResult::Skip);
        }

        let execution = runner::run(&self.deployment, &testcase.program);

        let oracle = SetupConnectionOracle;
        let result = match oracle.evaluate(&self.deployment, &testcase.program, &execution) {
            OracleResult::Pass => ScenarioResult::Ok,
            OracleResult::Fail(e) => ScenarioResult::Fail(format!("{}: {e}", oracle.name())),
        };
        (execution, result)
    }
}

impl Scenario<TestCase> for PoolSetupConnectionScenario {
    /// The seed has no effect: the pool runs on its own runtime, not on the simulator.
    fn new(_seed: u64) -> Result<Self> {
        Self::start()
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        self.execute(&testcase).1
    }
}
