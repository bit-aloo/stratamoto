use crate::error::Result;

pub enum ScenarioResult {
    Ok,
    /// The input does not describe a runnable test case.
    Skip,
    /// The deployment under test violated the protocol.
    Fail(String),
    /// The harness could not give the input a run: the deployment could not be brought up or
    /// reset. It says nothing about the input or the deployment's conformance, and nothing can
    /// run until it is fixed, so it is kept apart from a finding.
    Infrastructure(String),
}

pub trait ScenarioInput: Sized {
    fn decode(bytes: &[u8]) -> Result<Self>;
}

/// A reproducible test of an Sv2 deployment.
///
/// Whatever a run depends on travels with the input: a program carries the seed its simulated
/// deployment is built from, so there is nothing to read from the environment.
pub trait Scenario<I: ScenarioInput>: Sized {
    fn new() -> Result<Self>;
    fn run(&mut self, input: I) -> ScenarioResult;
}

#[macro_export]
macro_rules! stratamoto_main {
    ($scenario:ty, $input:ty) => {
        fn main() -> std::process::ExitCode {
            use $crate::scenario::{Scenario, ScenarioInput, ScenarioResult};

            // One subscriber for the harness and the real roles alike, filtered by RUST_LOG;
            // the runtime's `log` records are forwarded to it as well.
            $crate::tracing_subscriber::fmt()
                .with_env_filter($crate::tracing_subscriber::EnvFilter::from_default_env())
                .with_writer(std::io::stderr)
                .init();

            let bytes = match std::env::var("STRATAMOTO_INPUT") {
                Ok(path) => std::fs::read(path).unwrap_or_default(),
                Err(_) => {
                    use std::io::Read;
                    let mut bytes = Vec::new();
                    std::io::stdin().read_to_end(&mut bytes).unwrap_or_default();
                    bytes
                }
            };

            // Input that is not a test case is skipped rather than failed, as a fuzzer's
            // harness would; the reason is logged so that an artifact this build cannot read
            // says why, rather than passing in silence.
            let input = match <$input as ScenarioInput>::decode(&bytes) {
                Ok(input) => input,
                Err(e) => {
                    $crate::tracing::warn!("skipping input that does not decode: {e}");
                    return std::process::ExitCode::SUCCESS;
                }
            };

            // Exit codes: 0 for a pass or a skip, 1 for a finding, 2 when the harness could
            // not run the input at all, so that a script never reads an outage as a finding.
            let mut scenario = match <$scenario as Scenario<$input>>::new() {
                Ok(scenario) => scenario,
                Err(e) => {
                    $crate::tracing::error!(
                        "infrastructure failure: could not initialize the scenario: {e}"
                    );
                    return std::process::ExitCode::from(2);
                }
            };

            match scenario.run(input) {
                ScenarioResult::Ok => std::process::ExitCode::SUCCESS,
                ScenarioResult::Skip => {
                    $crate::tracing::warn!("skipping test case");
                    std::process::ExitCode::SUCCESS
                }
                ScenarioResult::Fail(e) => {
                    $crate::tracing::error!("test case failed: {e}");
                    std::process::ExitCode::FAILURE
                }
                ScenarioResult::Infrastructure(e) => {
                    $crate::tracing::error!("infrastructure failure: {e}");
                    std::process::ExitCode::from(2)
                }
            }
        }
    };
}
