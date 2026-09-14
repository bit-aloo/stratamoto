use stratamoto::{
    error::{Error, Result},
    oracle::{MiningChannelOracle, Oracle, OracleResult},
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

    /// Whether the pool is still serving.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.deployment.is_alive()
    }

    pub fn execute(&self, testcase: &TestCase) -> (Execution, ScenarioResult) {
        // A program written for more roles than the pool deployment has would open connections
        // to nothing, which is a mismatch with the input rather than a finding about the pool.
        if testcase.program.context.num_roles > self.num_roles() {
            return (Execution::default(), ScenarioResult::Skip);
        }

        let execution = runner::run(&self.deployment, &testcase.program);

        // The mining oracle applies here and not to the mock roles, which do not implement
        // mining: an unanswered channel open says something about a pool and nothing about a
        // role that was never asked to serve one.
        let oracle = MiningChannelOracle;
        if let OracleResult::Fail(e) = oracle.evaluate(&self.deployment, &testcase.program, &execution)
        {
            let result = ScenarioResult::Fail(format!("{}: {e}", oracle.name()));
            return (execution, result);
        }

        let result =
            crate::setup_connection::evaluate(&self.deployment, &testcase.program, &execution);
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
