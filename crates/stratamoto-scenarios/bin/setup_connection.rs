use stratamoto::{
    deployment::Deployment,
    error::{Error, Result},
    oracle::{Oracle, OracleResult, SetupConnectionOracle},
    roles::RoleConfig,
    runner,
    scenario::{Scenario, ScenarioInput, ScenarioResult},
    stratamoto_main,
    stratum_core::common_messages_sv2::Protocol,
};
use stratamoto_ir::{
    Program,
    compiler::{CompiledProgram, Compiler},
};

/// The deployment programs are run against: one upstream per subprotocol.
fn roles() -> Vec<RoleConfig> {
    vec![
        RoleConfig::new(Protocol::MiningProtocol),
        RoleConfig::new(Protocol::JobDeclarationProtocol),
        RoleConfig::new(Protocol::TemplateDistributionProtocol).with_supported_flags(0b1),
    ]
}

struct TestCase {
    program: CompiledProgram,
}

impl ScenarioInput for TestCase {
    fn decode(bytes: &[u8]) -> Result<Self> {
        let program: Program =
            postcard::from_bytes(bytes).map_err(|e| Error::Input(e.to_string()))?;
        if !program.is_statically_valid() {
            return Err(Error::Input("program is not statically valid".to_string()));
        }
        let program = Compiler::new()
            .compile(&program)
            .map_err(|e| Error::Input(e.to_string()))?;
        Ok(Self { program })
    }
}

struct SetupConnectionScenario {
    seed: u64,
}

impl Scenario<TestCase> for SetupConnectionScenario {
    fn new(seed: u64) -> Result<Self> {
        Ok(Self { seed })
    }

    fn run(&mut self, testcase: TestCase) -> ScenarioResult {
        let deployment = Deployment::new(self.seed, roles());
        let execution = runner::run(&deployment, &testcase.program);

        let oracle = SetupConnectionOracle;
        match oracle.evaluate(&deployment, &testcase.program, &execution) {
            OracleResult::Pass => ScenarioResult::Ok,
            OracleResult::Fail(e) => ScenarioResult::Fail(format!("{}: {e}", oracle.name())),
        }
    }
}

stratamoto_main!(SetupConnectionScenario, TestCase);
