//! Where a scenario's input comes from and where its verdict goes.
//!
//! Locally, the input is a file or stdin and the verdict is the exit code. Under Nyx, the
//! input comes from the fuzzer through the agent, asking for it takes the snapshot, and the
//! verdict is a hypercall: a failure is reported with its message, a skip restores the
//! snapshot without keeping the input, and a pass restores it when the runner is dropped.

use std::io::Read;

/// The environment variable naming the file a local run reads its input from.
pub const INPUT_ENV: &str = "STRATAMOTO_INPUT";

pub trait Runner {
    fn new() -> Self;
    /// The bytes of the test case to run. Under Nyx, the first call takes the snapshot.
    fn get_fuzz_input(&self) -> Vec<u8>;
    /// Report a finding.
    fn fail(&self, message: &str);
    /// Report that the input was not a test case, or not one for this scenario.
    fn skip(&self);
}

/// Reads the input from `STRATAMOTO_INPUT` or stdin, and reports through the log.
pub struct LocalRunner;

impl Runner for LocalRunner {
    fn new() -> Self {
        Self
    }

    fn get_fuzz_input(&self) -> Vec<u8> {
        match std::env::var(INPUT_ENV) {
            Ok(path) => {
                tracing::info!("reading the input from {path}");
                std::fs::read(&path).unwrap_or_default()
            }
            Err(_) => {
                tracing::info!("reading the input from stdin");
                let mut bytes = Vec::new();
                std::io::stdin().read_to_end(&mut bytes).unwrap_or_default();
                bytes
            }
        }
    }

    fn fail(&self, message: &str) {
        tracing::error!("{message}");
    }

    fn skip(&self) {
        tracing::warn!("skipping the test case");
    }
}

/// Talks to the fuzzer through the Nyx agent. Only meaningful inside a Nyx VM.
#[cfg(feature = "nyx")]
pub struct NyxRunner {
    max_input_size: usize,
}

#[cfg(feature = "nyx")]
impl Runner for NyxRunner {
    fn new() -> Self {
        // SAFETY: the agent is initialized once, here, before any other call into it.
        let max_input_size = unsafe { stratamoto_nyx_sys::nyx_init() };
        Self { max_input_size }
    }

    fn get_fuzz_input(&self) -> Vec<u8> {
        let mut data = vec![0u8; self.max_input_size];
        // SAFETY: `data` is as large as the agent was told the largest input is.
        let len = unsafe { stratamoto_nyx_sys::nyx_get_fuzz_input(data.as_ptr(), data.len()) };
        data.truncate(len);
        data
    }

    fn fail(&self, message: &str) {
        let message = std::ffi::CString::new(message).unwrap_or_default();
        // SAFETY: the message is a valid C string for the duration of both calls. It is
        // printed as well as reported, since the fuzzer reads messages from the print stream
        // and not from the report.
        unsafe {
            stratamoto_nyx_sys::nyx_println(message.as_ptr(), message.count_bytes());
            stratamoto_nyx_sys::nyx_fail(message.as_ptr());
        }
    }

    fn skip(&self) {
        // SAFETY: the agent is initialized.
        unsafe { stratamoto_nyx_sys::nyx_skip() }
    }
}

#[cfg(feature = "nyx")]
impl Drop for NyxRunner {
    fn drop(&mut self) {
        // SAFETY: the agent is initialized. A runner going out of scope is a run that ended
        // without a report, which is a pass; the snapshot is restored for the next input.
        unsafe { stratamoto_nyx_sys::nyx_release() }
    }
}

#[cfg(feature = "nyx")]
type DefaultRunner = NyxRunner;
#[cfg(not(feature = "nyx"))]
type DefaultRunner = LocalRunner;

/// The runner a scenario binary is built with: Nyx with the `nyx` feature, local otherwise.
pub struct StdRunner {
    runner: DefaultRunner,
}

impl Runner for StdRunner {
    fn new() -> Self {
        Self {
            runner: DefaultRunner::new(),
        }
    }

    fn get_fuzz_input(&self) -> Vec<u8> {
        self.runner.get_fuzz_input()
    }

    fn fail(&self, message: &str) {
        self.runner.fail(message);
    }

    fn skip(&self) {
        self.runner.skip();
    }
}

/// Write `bytes` where the fuzzer can read them, under `name`.
///
/// Under Nyx the file lands in the fuzzer's work directory, by hypercall. Locally it is
/// written to the path `STRATAMOTO_DUMP_<NAME>` names, when set, and dropped otherwise.
pub fn dump_to_host(name: &str, bytes: &[u8]) {
    #[cfg(feature = "nyx")]
    {
        // SAFETY: both buffers outlive the call, and their lengths are theirs.
        unsafe {
            stratamoto_nyx_sys::nyx_dump_file_to_host(
                name.as_ptr().cast(),
                name.len(),
                bytes.as_ptr(),
                bytes.len(),
            );
        }
    }
    #[cfg(not(feature = "nyx"))]
    {
        let variable: String = format!("STRATAMOTO_DUMP_{}", name.to_ascii_uppercase())
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if let Ok(path) = std::env::var(&variable)
            && let Err(e) = std::fs::write(&path, bytes)
        {
            tracing::warn!("could not write {name} to {path}: {e}");
        }
    }
}
