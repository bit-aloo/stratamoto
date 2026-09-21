//! Feedback from inside the target, alongside what a run did through the protocol.
//!
//! The behaviour signature says what a run drew out of the target; an observer says what it
//! touched in there. The two coexist: a program earns a corpus slot by being new to either.

/// What a run touched inside the target.
pub trait Observer {
    /// Clear what the previous run left, so that the next run's record is its own.
    fn reset(&mut self);

    /// Take in what the run touched. Returns whether it reached anything no earlier run had.
    fn observe(&mut self) -> bool;

    /// How much any run has reached so far, in the observer's own unit.
    fn seen(&self) -> usize;

    fn name(&self) -> &'static str;
}

/// No feedback from inside the target, which is the case for a binary that is not
/// instrumented: the behaviour signature is then all there is.
pub struct NoObserver;

impl Observer for NoObserver {
    fn reset(&mut self) {}

    fn observe(&mut self) -> bool {
        false
    }

    fn seen(&self) -> usize {
        0
    }

    fn name(&self) -> &'static str {
        "none"
    }
}

#[cfg(stratamoto_coverage)]
pub use llvm::LlvmCoverage;

/// Region coverage from the compiler's own instrumentation, read in process.
///
/// Built with `-C instrument-coverage`, every crate in the binary, the roles under test
/// included, counts how often each of its code regions ran, in one array the profiler runtime
/// owns. The observer reads that array and resets it, so no profile file, VM or snapshot is
/// involved; the build script turns this module on when it sees the flag.
#[cfg(stratamoto_coverage)]
mod llvm {
    use super::Observer;

    // The profiler runtime that `-C instrument-coverage` links in. The counters are 64 bit
    // and live in one section, which these bound.
    unsafe extern "C" {
        fn __llvm_profile_begin_counters() -> *const u8;
        fn __llvm_profile_end_counters() -> *const u8;
        fn __llvm_profile_reset_counters();
    }

    pub struct LlvmCoverage {
        counters: *const u64,
        len: usize,
        /// Which regions any run has reached.
        seen: Vec<bool>,
        seen_count: usize,
    }

    impl Default for LlvmCoverage {
        fn default() -> Self {
            Self::new()
        }
    }

    impl LlvmCoverage {
        #[must_use]
        pub fn new() -> Self {
            // SAFETY: both pointers bound the counter section the profiler runtime owns for
            // the life of the process, and counters are 8 byte aligned.
            let (counters, len) = unsafe {
                let begin = __llvm_profile_begin_counters().cast::<u64>();
                let end = __llvm_profile_end_counters().cast::<u64>();
                (begin, usize::try_from(end.offset_from(begin)).unwrap_or(0))
            };
            Self {
                counters,
                len,
                seen: vec![false; len],
                seen_count: 0,
            }
        }

        /// Whether region `index` ran since the last reset.
        ///
        /// Other threads, the roles' among them, bump counters without synchronization, so a
        /// read is volatile and a value only ever approximate; zero against not zero is all
        /// that is asked of it.
        fn hit(&self, index: usize) -> bool {
            // SAFETY: `index` is below the length the section bounds gave.
            unsafe { self.counters.add(index).read_volatile() != 0 }
        }

        /// The regions that ran since the last reset.
        #[must_use]
        pub fn hits(&self) -> Vec<usize> {
            (0..self.len).filter(|&i| self.hit(i)).collect()
        }
    }

    impl Observer for LlvmCoverage {
        fn reset(&mut self) {
            // SAFETY: the runtime's own reset, safe to call at any time.
            unsafe { __llvm_profile_reset_counters() }
        }

        fn observe(&mut self) -> bool {
            let mut new = false;
            for index in 0..self.len {
                if !self.seen[index] && self.hit(index) {
                    self.seen[index] = true;
                    self.seen_count += 1;
                    new = true;
                }
            }
            new
        }

        fn seen(&self) -> usize {
            self.seen_count
        }

        fn name(&self) -> &'static str {
            "llvm-coverage"
        }
    }
}
