//! `stratamoto init`: a Nyx share directory for a scenario.
//!
//! The share directory is what a Nyx VM is booted from: every file the scenario needs in the
//! VM, the packer's helpers that fetch them in, the VM's configuration, and the script Nyx
//! runs at boot. Adapted from fuzzamoto's `init`.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

pub struct InitArgs {
    /// Where to create the share directory. Must not exist.
    pub sharedir: PathBuf,
    /// The scenario binary, built with the `nyx` feature.
    pub scenario: PathBuf,
    /// The pool binary the scenario runs.
    pub pool: PathBuf,
    /// The directory sv2-apps' launcher keeps Bitcoin Core in.
    pub template_provider: PathBuf,
    /// A directory holding Nyx's `packer`: AFL++'s `nyx_mode`, or a target directory
    /// `libafl_nyx` built into.
    pub nyx_dir: PathBuf,
    /// The crash handler to preload into the pool.
    pub crash_handler: PathBuf,
    /// The VM's memory, in megabytes.
    pub memory: u32,
}

/// What the VM fetches from the share directory: files copied there by their base name.
#[derive(Default)]
struct Contents {
    files: BTreeSet<String>,
    executables: BTreeSet<String>,
}

impl Contents {
    fn add(&mut self, sharedir: &Path, source: &Path, executable: bool) -> Result<String, String> {
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| format!("{} has no usable file name", source.display()))?
            .to_string();
        std::fs::copy(source, sharedir.join(&name))
            .map_err(|e| format!("could not copy {}: {e}", source.display()))?;
        self.files.insert(name.clone());
        if executable {
            self.executables.insert(name.clone());
        }
        Ok(name)
    }
}

pub fn execute(args: &InitArgs) -> Result<(), String> {
    if args.sharedir.exists() {
        return Err(format!(
            "{} exists already; choose a fresh share directory",
            args.sharedir.display()
        ));
    }
    for (what, path) in [
        ("scenario", &args.scenario),
        ("pool", &args.pool),
        ("crash handler", &args.crash_handler),
        ("template provider directory", &args.template_provider),
        ("nyx directory", &args.nyx_dir),
    ] {
        if !path.exists() {
            return Err(format!("the {what} {} does not exist", path.display()));
        }
    }
    std::fs::create_dir_all(&args.sharedir).map_err(|e| e.to_string())?;
    let sharedir = &args.sharedir;

    // The binaries, and everything they link against: the VM has nothing of its own.
    let mut contents = Contents::default();
    let scenario = contents.add(sharedir, &args.scenario, true)?;
    let pool = contents.add(sharedir, &args.pool, true)?;
    let crash_handler = contents.add(sharedir, &args.crash_handler, true)?;
    let node_binaries = template_provider_binaries(&args.template_provider)?;
    for binary in [&args.scenario, &args.pool]
        .into_iter()
        .chain(node_binaries.iter())
    {
        for dependency in shared_libraries(binary)? {
            contents.add(sharedir, &dependency, false)?;
        }
    }
    contents
        .executables
        .insert("ld-linux-x86-64.so.2".to_string());

    // The template provider directory as the launcher expects it, as one archive.
    pack_template_provider(
        &args.template_provider,
        &sharedir.join("template-provider.tar"),
    )?;

    // Nyx's own helpers and the VM configuration.
    let packer = args.nyx_dir.join("packer").join("packer");
    let userspace = packer.join("linux_x86_64-userspace");
    run("bash", &["compile_64.sh"], &userspace)?;
    for entry in std::fs::read_dir(userspace.join("bin64")).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        std::fs::copy(entry.path(), sharedir.join(entry.file_name()))
            .map_err(|e| format!("could not copy {}: {e}", entry.path().display()))?;
    }
    run(
        "python3",
        &[
            "nyx_config_gen.py",
            sharedir
                .to_str()
                .ok_or("the share directory path is not UTF-8")?,
            "Kernel",
            "-m",
            &args.memory.to_string(),
        ],
        &packer,
    )?;

    std::fs::write(
        sharedir.join("fuzz_no_pt.sh"),
        boot_script(&contents, &scenario, &pool, &crash_handler),
    )
    .map_err(|e| e.to_string())?;

    eprintln!("created the share directory {}", sharedir.display());
    Ok(())
}

/// The binaries sv2-apps' launcher runs from its template provider directory.
fn template_provider_binaries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut binaries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("bitcoin-") {
            for candidate in [
                entry.path().join("bin").join("bitcoind"),
                entry.path().join("libexec").join("bitcoin-node"),
            ] {
                if candidate.exists() {
                    binaries.push(candidate);
                }
            }
        }
    }
    if binaries.is_empty() {
        return Err(format!(
            "{} holds no bitcoin-*/bin/bitcoind or bitcoin-*/libexec/bitcoin-node",
            dir.display()
        ));
    }
    Ok(binaries)
}

/// One archive of the `bitcoin-*` entries, unpacked in the VM under `template-provider`.
fn pack_template_provider(dir: &Path, archive: &Path) -> Result<(), String> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        let name = name.to_string_lossy().into_owned();
        if name.starts_with("bitcoin-") {
            entries.push(name);
        }
    }
    let mut args = vec![
        "-cf".to_string(),
        archive.to_string_lossy().into_owned(),
        "-C".to_string(),
        dir.to_string_lossy().into_owned(),
    ];
    args.extend(entries);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    run("tar", &args, Path::new("."))
}

/// The shared libraries `binary` loads, including the dynamic loader.
///
/// `lddtree` from pax-utils resolves them the way the loader will; `ldd` is the fallback.
fn shared_libraries(binary: &Path) -> Result<Vec<PathBuf>, String> {
    let binary = binary.to_str().ok_or("a binary path is not UTF-8")?;
    let (tool, output) = match Command::new("lddtree").arg(binary).output() {
        Ok(output) if output.status.success() => ("lddtree", output),
        _ => (
            "ldd",
            Command::new("ldd")
                .arg(binary)
                .output()
                .map_err(|e| format!("neither lddtree nor ldd runs: {e}"))?,
        ),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut libraries = Vec::new();
    for line in text.lines().skip(usize::from(tool == "lddtree")) {
        let line = line.trim();
        // lddtree: "libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6"
        // ldd:     "libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x...)" and
        //          "/lib64/ld-linux-x86-64.so.2 (0x...)"
        let path = match line.split_once("=>") {
            Some((_, rest)) => rest.trim().split(' ').next().unwrap_or(""),
            None => line.split(' ').next().unwrap_or(""),
        };
        if path.starts_with('/') {
            libraries.push(PathBuf::from(path));
        }
    }
    if tool == "ldd"
        && !libraries
            .iter()
            .any(|l| l.ends_with("ld-linux-x86-64.so.2"))
    {
        return Err(format!("could not find the dynamic loader of {binary}"));
    }
    Ok(libraries)
}

fn run(program: &str, args: &[&str], dir: &Path) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed with {status}", args.join(" ")))
    }
}

/// The script Nyx runs when the VM boots: fetch everything in, bring the loopback up, and
/// start the scenario with the pool wrapped so that the crash handler is preloaded into it.
fn boot_script(contents: &Contents, scenario: &str, pool: &str, crash_handler: &str) -> String {
    let mut script = vec![
        "chmod +x hget".to_string(),
        "cp hget /tmp".to_string(),
        "cd /tmp".to_string(),
        "echo 0 > /proc/sys/kernel/randomize_va_space".to_string(),
        "echo 0 > /proc/sys/kernel/printk".to_string(),
        "./hget hcat_no_pt hcat".to_string(),
        "./hget habort_no_pt habort".to_string(),
    ];
    for file in &contents.files {
        script.push(format!("./hget {file} {file}"));
    }
    script.push("./hget template-provider.tar template-provider.tar".to_string());
    script.push("mkdir -p template-provider".to_string());
    script.push("tar -xf template-provider.tar -C template-provider".to_string());
    script.push("chmod +x habort hcat".to_string());
    for executable in &contents.executables {
        script.push(format!("chmod +x {executable}"));
    }
    script.push("chmod +x template-provider/*/bin/* template-provider/*/libexec/*".to_string());

    script.push("export __AFL_DEFER_FORKSRV=1".to_string());
    script.push("ip addr add 127.0.0.1/8 dev lo".to_string());
    script.push("ip link set lo up".to_string());
    script.push("ip a | ./hcat".to_string());

    // The pool runs behind a proxy script so that the crash handler is preloaded into it and
    // not into the scenario.
    script.push("echo '#!/bin/sh' > ./pool_proxy".to_string());
    script.push(format!(
        "echo 'LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 LD_PRELOAD=./{crash_handler} ./{pool} \"$@\"' >> ./pool_proxy"
    ));
    script.push("chmod +x ./pool_proxy".to_string());

    script.push(format!(
        "RUST_LOG=info LD_LIBRARY_PATH=/tmp LD_BIND_NOW=1 ./{scenario} ./pool_proxy > log.txt 2>&1"
    ));
    script.push("cat log.txt | ./hcat".to_string());
    script.push(
        "./habort \"target has terminated without initializing the fuzzing agent ...\"".to_string(),
    );
    script.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boot_script_fetches_everything_and_wraps_the_pool() {
        let mut contents = Contents::default();
        contents.files.insert("libc.so.6".to_string());
        contents.files.insert("pool_sv2".to_string());
        contents.executables.insert("pool_sv2".to_string());
        let script = boot_script(&contents, "pool_setup_connection", "pool_sv2", "handler.so");

        assert!(script.contains("./hget libc.so.6 libc.so.6\n"));
        assert!(script.contains("chmod +x pool_sv2\n"));
        assert!(script.contains("tar -xf template-provider.tar -C template-provider\n"));
        assert!(script.contains("LD_PRELOAD=./handler.so ./pool_sv2 \"$@\"' >> ./pool_proxy\n"));
        assert!(script.contains("./pool_setup_connection ./pool_proxy > log.txt 2>&1\n"));
    }

    #[test]
    fn the_binaries_of_a_template_provider_directory_are_found() {
        let dir = std::env::temp_dir().join(format!("stratamoto-tp-{}", std::process::id()));
        for path in [
            "bitcoin-31.0/bin/bitcoind",
            "bitcoin-31.0/libexec/bitcoin-node",
            "sv2-tp-1.1.0/bin/sv2-tp",
        ] {
            let path = dir.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
        }
        std::fs::create_dir_all(dir.join(".locks")).unwrap();

        let mut found: Vec<String> = template_provider_binaries(&dir)
            .unwrap()
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().into_owned())
            .collect();
        found.sort();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            found,
            vec![
                "bitcoin-31.0/bin/bitcoind",
                "bitcoin-31.0/libexec/bitcoin-node"
            ]
        );
    }

    #[test]
    fn the_loader_and_libraries_of_a_binary_are_listed() {
        let libraries = shared_libraries(Path::new("/bin/sh")).unwrap();
        assert!(
            libraries
                .iter()
                .any(|l| l.ends_with("ld-linux-x86-64.so.2"))
        );
        assert!(
            libraries
                .iter()
                .any(|l| l.to_string_lossy().contains("libc.so"))
        );
    }
}
