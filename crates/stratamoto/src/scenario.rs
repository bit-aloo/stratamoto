use crate::error::Result;

pub enum ScenarioResult {
    Ok,
    /// The input does not describe a runnable test case.
    Skip,
    /// The deployment under test violated the protocol. The text starts with the name of the
    /// oracle that found it, then `: ` and the detail.
    Fail(String),
    /// The harness could not give the input a run. It says nothing about the input or the
    /// deployment's conformance, so it is kept apart from a finding.
    Infrastructure(String),
}

pub trait ScenarioInput: Sized {
    fn decode(bytes: &[u8]) -> Result<Self>;
}

/// A reproducible test of an Sv2 deployment.
///
/// `new` brings up what every test case shares, from the command line the binary was given:
/// under a snapshotting fuzzer, everything it does is in the snapshot every input starts
/// from. `run` runs one test case against it.
pub trait Scenario<I: ScenarioInput>: Sized {
    fn new(args: &[String]) -> Result<Self>;
    fn run(&mut self, input: I) -> ScenarioResult;
}

/// The entry point of a scenario binary.
///
/// The scenario is set up, then the runner is asked for the input, which under Nyx is where
/// the snapshot is taken; the input is decoded and run, and the result reported through the
/// runner: a failure with its message, a skip as a skip, and a pass by returning. Locally the
/// exit code is 0 for a pass or a skip, 1 for a finding and 2 when the harness could not run
/// the input at all, so that a script never reads an outage as a finding.
#[macro_export]
macro_rules! stratamoto_main {
    ($scenario:ty, $input:ty) => {
        fn main() -> std::process::ExitCode {
            use $crate::{
                runners::{Runner, StdRunner},
                scenario::{Scenario, ScenarioInput, ScenarioResult},
            };

            // One subscriber for the harness and the real roles alike, filtered by RUST_LOG.
            $crate::tracing_subscriber::fmt()
                .with_env_filter($crate::tracing_subscriber::EnvFilter::from_default_env())
                .with_writer(std::io::stderr)
                .init();

            let runner = StdRunner::new();

            let args: Vec<String> = std::env::args().collect();
            let mut scenario = match <$scenario as Scenario<$input>>::new(&args) {
                Ok(scenario) => scenario,
                Err(e) => {
                    runner.fail(&format!(
                        "INFRASTRUCTURE: could not initialize the scenario: {e}"
                    ));
                    return std::process::ExitCode::from(2);
                }
            };

            // Under Nyx the snapshot is taken here, and every input starts from it.
            let bytes = runner.get_fuzz_input();

            // Input that is not a test case is skipped rather than failed; the reason is
            // logged so that a file this build cannot read says why.
            let input = match <$input as ScenarioInput>::decode(&bytes) {
                Ok(input) => input,
                Err(e) => {
                    $crate::tracing::warn!("the input does not decode: {e}");
                    runner.skip();
                    return std::process::ExitCode::SUCCESS;
                }
            };

            match scenario.run(input) {
                ScenarioResult::Ok => {
                    $crate::tracing::info!("the test case passed");
                    std::process::ExitCode::SUCCESS
                }
                ScenarioResult::Skip => {
                    runner.skip();
                    std::process::ExitCode::SUCCESS
                }
                ScenarioResult::Fail(e) => {
                    runner.fail(&format!("FAIL: {e}"));
                    std::process::ExitCode::FAILURE
                }
                ScenarioResult::Infrastructure(e) => {
                    runner.fail(&format!("INFRASTRUCTURE: {e}"));
                    std::process::ExitCode::from(2)
                }
            }
        }
    };
}
