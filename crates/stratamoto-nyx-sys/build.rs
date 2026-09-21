//! Compiles the Nyx agent into the crate, and the crash handler into a shared object a target
//! can be preloaded with.
//!
//! The agent needs to know the size of the target's AFL coverage map at compile time, so that
//! the shared map it hands the target and the fuzzer is that large. An AFL instrumented binary
//! prints its map size when run with `AFL_DUMP_MAP_SIZE=1`; the pool binary named by
//! `STRATAMOTO_POOL` is asked, and when it is not instrumented, or not named, the agent falls
//! back to the size the host supplies.

use std::{path::PathBuf, process::Command};

const POOL_BINARY_ENV: &str = "STRATAMOTO_POOL";

/// The AFL coverage map size of an instrumented binary, or `None` when it is not one.
fn map_size(binary: &PathBuf) -> Option<String> {
    let output = Command::new(binary)
        .env("AFL_DUMP_MAP_SIZE", "1")
        .output()
        .ok()?;
    let size = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!size.is_empty() && size.chars().all(|c| c.is_ascii_digit())).then_some(size)
}

fn main() {
    println!("cargo:rerun-if-changed=src/nyx-agent.c");
    println!("cargo:rerun-if-changed=src/nyx-crash-handler.c");
    println!("cargo:rerun-if-changed=src/nyx.h");
    println!("cargo:rerun-if-env-changed={POOL_BINARY_ENV}");

    let mut agent = cc::Build::new();
    agent
        .file("src/nyx-agent.c")
        .define("NO_PT_NYX", None)
        // The header defines helpers the agent does not call.
        .flag("-Wno-unused-function");
    if let Some(binary) = std::env::var_os(POOL_BINARY_ENV) {
        let binary = PathBuf::from(binary);
        println!("cargo:rerun-if-changed={}", binary.display());
        match map_size(&binary) {
            Some(size) => {
                println!("cargo:warning=nyx agent built for a target map of {size} bytes");
                agent.define("TARGET_MAP_SIZE", size.as_str());
            }
            None => println!(
                "cargo:warning={} is not AFL instrumented; the nyx agent will use the host's map size",
                binary.display()
            ),
        }
    }
    agent.compile("nyx_agent");

    // The crash handler is preloaded into the target, so it is a shared object of its own
    // rather than part of this crate. The compiler cc found is used, so that it matches.
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let handler = out.join("libstratamoto_crash_handler.so");
    let compiler = cc::Build::new().get_compiler();
    let status = compiler
        .to_command()
        .args([
            "-fPIC",
            "-shared",
            "-DENABLE_NYX",
            "-D_GNU_SOURCE",
            "-DNO_PT_NYX",
        ])
        .arg("src/nyx-crash-handler.c")
        .arg("-ldl")
        .arg("-o")
        .arg(&handler)
        .status()
        .expect("the C compiler runs");
    assert!(status.success(), "the crash handler compiles");
    println!(
        "cargo:rustc-env=STRATAMOTO_CRASH_HANDLER={}",
        handler.display()
    );
}
