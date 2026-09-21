//! Bindings to the Nyx agent, the C code a scenario talks to the snapshotting VM through.
//!
//! The agent is fuzzamoto's, vendored: see `LICENSE-fuzzamoto`. It is only meaningful inside a
//! Nyx VM; outside one the hypercalls fault, so a scenario built with it cannot be run
//! locally.

use std::os::raw::{c_char, c_uchar};

/// The crash handler, built as a shared object for preloading into the target.
///
/// A target that aborts or fails an assertion with this preloaded reports the crash to Nyx
/// directly, with the log it collected, rather than only dying.
pub const CRASH_HANDLER: &str = env!("STRATAMOTO_CRASH_HANDLER");

unsafe extern "C" {
    /// Set the agent up: check the host, create the shared coverage map and hand it to the
    /// target through `__AFL_SHM_ID`. Returns the largest input the host will supply.
    pub fn nyx_init() -> usize;
    /// Write `data` to a file named `file_name` on the host, appending.
    pub fn nyx_dump_file_to_host(
        file_name: *const c_char,
        file_name_len: usize,
        data: *const c_uchar,
        len: usize,
    );
    /// Copy the next input into `data` and return its length. The first call takes the
    /// snapshot: everything before it is what every input starts from.
    pub fn nyx_get_fuzz_input(data: *const c_uchar, max_size: usize) -> usize;
    /// Reset the coverage map and restore the snapshot, discarding this input.
    pub fn nyx_skip();
    /// Restore the snapshot.
    pub fn nyx_release();
    /// Report a crash with `message` and restore the snapshot.
    pub fn nyx_fail(message: *const c_char);
    /// Print through the hypervisor, where the fuzzer can read it.
    pub fn nyx_println(message: *const c_char, size: usize);
}
