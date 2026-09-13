use crate::error::Result;

pub enum ScenarioResult {
    Ok,
    /// The input does not describe a runnable test case.
    Skip,
    /// The deployment under test violated the protocol.
    Fail(String),
}

pub trait ScenarioInput: Sized {
    fn decode(bytes: &[u8]) -> Result<Self>;
}

/// A reproducible test of an Sv2 deployment, run on a seeded simulator.
pub trait Scenario<I: ScenarioInput>: Sized {
    fn new(seed: u64) -> Result<Self>;
    fn run(&mut self, input: I) -> ScenarioResult;
}

#[macro_export]
macro_rules! stratamoto_main {
    ($scenario:ty, $input:ty) => {
        fn main() -> std::process::ExitCode {
            use $crate::scenario::{Scenario, ScenarioInput, ScenarioResult};

            env_logger::init();

            let seed = std::env::var("STRATAMOTO_SEED")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            let bytes = match std::env::var("STRATAMOTO_INPUT") {
                Ok(path) => std::fs::read(path).unwrap_or_default(),
                Err(_) => {
                    use std::io::Read;
                    let mut bytes = Vec::new();
                    std::io::stdin().read_to_end(&mut bytes).unwrap_or_default();
                    bytes
                }
            };

            let Ok(input) = <$input as ScenarioInput>::decode(&bytes) else {
                log::warn!("failed to decode input");
                return std::process::ExitCode::SUCCESS;
            };

            let mut scenario = match <$scenario as Scenario<$input>>::new(seed) {
                Ok(scenario) => scenario,
                Err(e) => {
                    log::error!("failed to initialize scenario: {e}");
                    return std::process::ExitCode::FAILURE;
                }
            };

            match scenario.run(input) {
                ScenarioResult::Ok => std::process::ExitCode::SUCCESS,
                ScenarioResult::Skip => {
                    log::warn!("skipping test case");
                    std::process::ExitCode::SUCCESS
                }
                ScenarioResult::Fail(e) => {
                    log::error!("test case failed: {e}");
                    std::process::ExitCode::FAILURE
                }
            }
        }
    };
}
