//! The `macros` feature re-exports `stratamoto_macros::test`, which wraps an async
//! function in a seeded `Runtime` and runs it under `block_on`.
#![cfg(feature = "macros")]

use std::time::Duration;

use stratamoto_dst::{
    Handle,
    time::{self, Instant},
};

#[stratamoto_dst::test]
async fn runs_the_body_on_a_runtime() {
    // The macro entered the runtime, so a handle is available and simulated time advances.
    let _handle = Handle::current();
    let before = Instant::now();
    time::sleep(Duration::from_secs(5)).await;
    assert!(before.elapsed() >= Duration::from_secs(5));
}
