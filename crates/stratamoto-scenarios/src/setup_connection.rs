use stratamoto::{
    error::{Error, Result},
    oracle::{CrashOracle, HarnessIntegrityOracle, Oracle, OracleResult, SetupConnectionOracle},
    runner::Execution,
    scenario::{ScenarioInput, ScenarioResult},
    transport::Deployment,
};
use stratamoto_ir::{
    Program,
    compiler::{CompiledProgram, Compiler},
};

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
    /// A program as the CLI writes one and the fuzzer keeps them.
    fn decode(bytes: &[u8]) -> Result<Self> {
        let program: Program =
            postcard::from_bytes(bytes).map_err(|e| Error::Input(e.to_string()))?;
        Self::from_program(&program)
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
