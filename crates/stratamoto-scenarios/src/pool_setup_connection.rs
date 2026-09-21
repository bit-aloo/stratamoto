use std::path::Path;

use stratamoto::{
    error::{Error, Result},
    runner::{self, Execution},
    runners::dump_to_host,
    scenario::{Scenario, ScenarioResult},
    transport::Deployment,
};
use stratamoto_ir::ProgramContext;
use stratamoto_targets::pool::PoolDeployment;

use crate::setup_connection::{Run, TestCase};

/// The name the program context is dumped under, for the fuzzer to generate programs against.
pub const CONTEXT_DUMP: &str = "ir.context";

/// The setup connection scenario against sv2-apps' pool.
///
/// One pool serves the whole life of the scenario. What keeps test cases apart is that a
/// scenario process runs one of them: locally, one input per process; under a snapshotting
/// fuzzer, one input per restore of the snapshot taken after the pool came up.
pub struct PoolSetupConnectionScenario {
    deployment: PoolDeployment,
}

impl PoolSetupConnectionScenario {
    /// Bring the pool up, with the node behind it, from the binary
    /// `STRATAMOTO_POOL` names.
    pub fn start() -> Result<Self> {
        let deployment = PoolDeployment::start().map_err(|e| Error::Target(e.to_string()))?;
        Ok(Self::over(deployment))
    }

    /// Bring the pool up from the binary at `pool`.
    pub fn start_with(pool: &Path) -> Result<Self> {
        let deployment =
            PoolDeployment::start_with(pool).map_err(|e| Error::Target(e.to_string()))?;
        Ok(Self::over(deployment))
    }

    fn over(deployment: PoolDeployment) -> Self {
        let scenario = Self { deployment };
        scenario.dump_context();
        scenario
    }

    /// The context programs for this deployment are written in.
    #[must_use]
    pub fn context(&self) -> ProgramContext {
        ProgramContext {
            num_roles: self.deployment.num_roles(),
            num_connections: 0,
            seed: 0,
        }
    }

    /// Hand the fuzzer the context, so that what it generates addresses the roles that exist.
    fn dump_context(&self) {
        match postcard::to_allocvec(&self.context()) {
            Ok(bytes) => dump_to_host(CONTEXT_DUMP, &bytes),
            Err(e) => tracing::warn!("could not encode the program context: {e}"),
        }
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
    /// The pool binary is the first argument, as a fuzzer's share directory passes it, or
    /// `STRATAMOTO_POOL` when there is none.
    fn new(args: &[String]) -> Result<Self> {
        match args.get(1) {
            Some(pool) => Self::start_with(Path::new(pool)),
            None => Self::start(),
        }
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        self.execute(&testcase).result
    }
}
