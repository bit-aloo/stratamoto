//! Records what the binary is built against, so that an artifact can say which revision of
//! the roles under test it was found on rather than "whatever the lock file said at the time",
//! and whether it is built with coverage instrumentation the fuzzer can observe.

use std::{fs, path::Path, process::Command};

/// The crates whose revision decides what a finding is about: the roles under test, their
/// launchers, and the protocol library both sides of a connection share.
const PINNED: [&str; 4] = [
    "pool_sv2",
    "stratum-apps",
    "integration_tests_sv2",
    "stratum-core",
];

fn main() {
    // A binary built with `-C instrument-coverage` carries a counter array the observer can
    // read in process; one built without has nothing to read and no runtime to link against,
    // so the observer exists only in the former.
    println!("cargo:rustc-check-cfg=cfg(stratamoto_coverage)");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    let flags = std::env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
    if flags.contains("instrument-coverage") {
        println!("cargo:rustc-cfg=stratamoto_coverage");
    }

    let root =
        Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it")).join("../..");

    let lock = root.join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    let mut revisions: Vec<String> = fs::read_to_string(&lock)
        .unwrap_or_default()
        .split("[[package]]")
        .filter_map(package)
        .filter(|(name, _, _)| PINNED.contains(&name.as_str()))
        .map(|(name, version, source)| format!("{name} {version} {source}"))
        .collect();

    // The harness itself. A commit changes the ref the head points at, a checkout changes the
    // head, and staging changes the index, so watching those three catches what matters.
    let git = root.join(".git");
    println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
    println!("cargo:rerun-if-changed={}", git.join("index").display());
    if let Some(reference) = fs::read_to_string(git.join("HEAD"))
        .ok()
        .and_then(|head| head.strip_prefix("ref: ").map(|r| r.trim().to_string()))
    {
        println!("cargo:rerun-if-changed={}", git.join(reference).display());
    }
    let head = output(&root, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = output(&root, &["status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    revisions.push(format!(
        "stratamoto {} git+local#{head}{}",
        std::env::var("CARGO_PKG_VERSION").expect("cargo sets it"),
        if dirty { "-dirty" } else { "" }
    ));

    println!(
        "cargo:rustc-env=STRATAMOTO_REVISIONS={}",
        revisions.join(";")
    );
}

/// The name, version and source of one `[[package]]` block of the lock file.
fn package(block: &str) -> Option<(String, String, String)> {
    let field = |key: &str| {
        block.lines().find_map(|line| {
            line.strip_prefix(key)?
                .strip_prefix(" = \"")?
                .strip_suffix('"')
                .map(str::to_string)
        })
    };
    Some((field("name")?, field("version")?, field("source")?))
}

fn output(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}
