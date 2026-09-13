use stratamoto::{
    deployment::Deployment,
    error::{Error, Result},
    oracle::{Oracle, OracleResult, SetupConnectionOracle},
    roles::RoleConfig,
    runner::{self, Execution, SetupResponse},
    scenario::{Scenario, ScenarioInput, ScenarioResult},
    stratum_core::common_messages_sv2::Protocol,
};
use stratamoto_ir::{
    Program,
    compiler::{CompiledProgram, Compiler},
};

/// The deployment programs are run against: one upstream per subprotocol.
///
/// Template distribution supports a feature flag so that flag negotiation, and not just
/// rejection, is reachable.
#[must_use]
pub fn roles() -> Vec<RoleConfig> {
    vec![
        RoleConfig::new(Protocol::MiningProtocol),
        RoleConfig::new(Protocol::JobDeclarationProtocol),
        RoleConfig::new(Protocol::TemplateDistributionProtocol).with_supported_flags(0b1),
    ]
}

pub struct TestCase {
    pub program: CompiledProgram,
}

impl TestCase {
    pub fn from_program(program: &Program) -> Result<Self> {
        if !program.is_statically_valid() {
            return Err(Error::Input("program is not statically valid".to_string()));
        }
        let program = Compiler::new()
            .compile(program)
            .map_err(|e| Error::Input(e.to_string()))?;
        Ok(Self { program })
    }
}

impl ScenarioInput for TestCase {
    fn decode(bytes: &[u8]) -> Result<Self> {
        let program: Program =
            postcard::from_bytes(bytes).map_err(|e| Error::Input(e.to_string()))?;
        Self::from_program(&program)
    }
}

pub struct SetupConnectionScenario {
    seed: u64,
}

impl SetupConnectionScenario {
    /// Run a test case and hand back what the deployment did, for a caller that wants more
    /// than a pass or fail.
    pub fn execute(&self, testcase: &TestCase) -> (Execution, ScenarioResult) {
        let deployment = Deployment::new(self.seed, roles());
        let execution = runner::run(&deployment, &testcase.program);

        let oracle = SetupConnectionOracle;
        let result = match oracle.evaluate(&deployment, &testcase.program, &execution) {
            OracleResult::Pass => ScenarioResult::Ok,
            OracleResult::Fail(e) => ScenarioResult::Fail(format!("{}: {e}", oracle.name())),
        };
        (execution, result)
    }
}

impl Scenario<TestCase> for SetupConnectionScenario {
    fn new(seed: u64) -> Result<Self> {
        Ok(Self { seed })
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        self.execute(&testcase).1
    }
}

/// A coarse summary of what a run did, used to tell a new behaviour from a repeat of one
/// already in the corpus.
///
/// It is the set of distinct interactions a run produced, not how many of each: a program
/// that opens the same session twice has not reached anywhere new, while one that draws an
/// error a role has not returned before has.
#[must_use]
pub fn signature(execution: &Execution) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut interactions: Vec<(usize, stratamoto_ir::Protocol, u8, u64)> = execution
        .sessions
        .values()
        .map(|session| {
            let role = execution
                .connection_roles
                .get(&session.connection)
                .copied()
                .unwrap_or(usize::MAX);
            let (kind, detail) = match &session.response {
                SetupResponse::Success { used_version, .. } => (0u8, u64::from(*used_version)),
                SetupResponse::Error { error_code, .. } => (1, hash(error_code)),
                SetupResponse::Unexpected { message_type } => (2, u64::from(*message_type)),
                SetupResponse::Silence => (3, 0),
            };
            (role, session.protocol, kind, detail)
        })
        .collect();
    interactions.sort();
    interactions.dedup();

    let mut hasher = std::hash::DefaultHasher::new();
    interactions.hash(&mut hasher);
    // Whether anything arrived unprompted, not how much of it.
    (!execution.unsolicited.is_empty()).hash(&mut hasher);
    hasher.finish()
}

fn hash<T: std::hash::Hash>(value: T) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::hash::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
