use stratamoto::{
    deployment::SimulatedDeployment,
    error::{Error, Result},
    oracle::{CrashOracle, HarnessIntegrityOracle, Oracle, OracleResult, SetupConnectionOracle},
    roles::RoleConfig,
    runner::{self, Execution},
    scenario::{Scenario, ScenarioInput, ScenarioResult},
    stratum_core::common_messages_sv2::Protocol,
    transport::Deployment,
};
use stratamoto_ir::{
    Program,
    artifact::read_program,
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

/// What running a test case produced: the record and the verdict.
pub struct Run {
    pub execution: Execution,
    pub result: ScenarioResult,
}

impl Run {
    /// Judge an execution with the scenario's oracles.
    pub fn of<D: Deployment>(
        deployment: &D,
        program: &CompiledProgram,
        execution: Execution,
        judge: impl FnOnce(&D, &CompiledProgram, &Execution) -> ScenarioResult,
    ) -> Self {
        let result = judge(deployment, program, &execution);
        Self { execution, result }
    }
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
    /// A saved artifact or a bare program, as the CLI writes one.
    fn decode(bytes: &[u8]) -> Result<Self> {
        let program = read_program(bytes).map_err(|e| Error::Input(e.to_string()))?;
        Self::from_program(&program)
    }
}

/// The setup connection scenario against the mock roles.
///
/// Every test case gets a fresh deployment, built from the seed its own program carries, so a
/// case cannot be affected by the ones before it and needs nothing but its bytes to replay.
pub struct SetupConnectionScenario;

impl SetupConnectionScenario {
    /// Run a test case and hand back what the deployment did, for a caller that wants more
    /// than a pass or fail.
    pub fn execute(&self, testcase: &TestCase) -> Run {
        let deployment = SimulatedDeployment::new(testcase.program.context.seed, roles());
        let execution = runner::run(&deployment, &testcase.program);
        Run::of(&deployment, &testcase.program, execution, evaluate)
    }
}

impl Scenario<TestCase> for SetupConnectionScenario {
    fn new() -> Result<Self> {
        Ok(Self)
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        self.execute(&testcase).result
    }
}

/// Run the scenario's oracles in order, reporting the first violation.
///
/// The harness's own integrity first, since a run the harness did not complete says nothing
/// about the deployment and is reported as such; then conformance, then liveness: a crash is
/// worth knowing about however the messages looked, so it is checked even when the answers
/// were within the specification.
pub fn evaluate<D: stratamoto::transport::Deployment>(
    deployment: &D,
    program: &stratamoto_ir::compiler::CompiledProgram,
    execution: &Execution,
) -> ScenarioResult {
    let integrity = HarnessIntegrityOracle;
    if let OracleResult::Fail(e) = integrity.evaluate(deployment, program, execution) {
        return ScenarioResult::Infrastructure(format!("{}: {e}", integrity.name()));
    }
    let conformance = SetupConnectionOracle;
    if let OracleResult::Fail(e) = conformance.evaluate(deployment, program, execution) {
        return ScenarioResult::Fail(format!("{}: {e}", conformance.name()));
    }
    let crash = CrashOracle;
    if let OracleResult::Fail(e) = crash.evaluate(deployment, program, execution) {
        return ScenarioResult::Fail(format!("{}: {e}", crash.name()));
    }
    ScenarioResult::Ok
}
