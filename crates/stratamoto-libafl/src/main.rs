//! The fuzzer: LibAFL clients, each driving a Nyx VM that runs a scenario, mutating IR
//! programs with the mutators and generators of `stratamoto-ir`.
//!
//! Ported from fuzzamoto-libafl. Only Linux on x86_64 can run Nyx.

mod client;
mod feedbacks;
mod fuzzer;
mod input;
mod instance;
mod monitor;
mod mutators;
mod options;
mod schedulers;
mod stages;

use crate::fuzzer::Fuzzer;

pub fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    Fuzzer::new().fuzz().unwrap();
}
