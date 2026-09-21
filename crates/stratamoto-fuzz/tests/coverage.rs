//! The coverage observer, on a binary built with `-C instrument-coverage`.
//!
//! Without the flag there is no counter array to read and the build script leaves the observer
//! out, so this file is empty then and the test runs only in an instrumented build.
#![cfg(stratamoto_coverage)]

use std::{collections::HashSet, hint::black_box};

use stratamoto_fuzz::observer::{LlvmCoverage, Observer};

/// A branch that one input reaches and the other does not.
#[inline(never)]
fn planted(input: u32) -> u32 {
    if input == 42 {
        black_box(input.wrapping_mul(3))
    } else {
        black_box(input)
    }
}

fn run(observer: &mut LlvmCoverage, input: u32) -> (bool, HashSet<usize>) {
    observer.reset();
    planted(black_box(input));
    let new = observer.observe();
    (new, observer.hits().into_iter().collect())
}

/// The observer sees a planted branch the first time an input reaches it and not the second,
/// and the next input starts from a cleared map rather than inherit the branch.
#[test]
fn in_process_coverage_smoke() {
    let mut observer = LlvmCoverage::new();
    assert!(observer.seen() == 0);

    // Warm up on the input that misses the branch, so that whatever the harness and the
    // observer touch on their first time through is already seen and the plain input is old.
    let mut plain = HashSet::new();
    for _ in 0..3 {
        plain.extend(run(&mut observer, 0).1);
    }
    assert!(
        !run(&mut observer, 0).0,
        "a plain input still counts as new"
    );
    let before = observer.seen();

    // The branch is new once.
    let (new, with_branch) = run(&mut observer, 42);
    assert!(new, "the planted branch was not observed");
    assert!(observer.seen() > before);
    let branch: Vec<usize> = with_branch.difference(&plain).copied().collect();
    assert!(!branch.is_empty(), "the branch left no region of its own");
    assert!(!run(&mut observer, 42).0, "the branch counted as new twice");

    // And it is cleared before the next input, which misses it.
    let (new, after) = run(&mut observer, 0);
    assert!(!new);
    let survived: Vec<usize> = branch
        .iter()
        .copied()
        .filter(|r| after.contains(r))
        .collect();
    assert!(
        survived.is_empty(),
        "regions {survived:?} of the branch survived the reset into a run that did not take it"
    );
}
