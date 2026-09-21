//! The lifecycle of a target that every input must start alike on.
//!
//! The boundary is independent of the IR and the oracles: a backend brings a deployment up,
//! records the state every input starts from, hands the deployment out for one input at a
//! time, and takes it back to that state afterwards. Restarting the target is the first
//! implementation; a full-system snapshot is the one it is shaped for.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{digest::Verdict, transport::Deployment};

/// What every input starts from, as an artifact records it.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SnapshotId {
    /// Which backend took it.
    pub backend: String,
    /// What it is a snapshot of: a policy for a restart backend, an image hash for a VM.
    pub identity: String,
}

impl fmt::Display for SnapshotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.backend, self.identity)
    }
}

/// A target that returns to one known state between inputs.
pub trait ExecutionBackend {
    type Deployment: Deployment;
    type Error: fmt::Display;

    /// Bring the target and whatever it depends on up. Once, before anything else.
    fn prepare_scenario(&mut self) -> Result<(), Self::Error>;

    /// Record the state every input starts from: after the dependencies are ready and the
    /// health checks pass, and before any input is consumed.
    fn take_root_snapshot(&mut self) -> Result<SnapshotId, Self::Error>;

    /// The deployment, at the root state, for one input.
    ///
    /// An error here is an infrastructure failure: the target could not be brought back to
    /// the root, or was found not to be there when checked.
    fn next_input(&mut self) -> Result<&Self::Deployment, Self::Error>;

    /// Take in what the input just run came to, and make ready to return to the root.
    fn report_and_reset(&mut self, verdict: &Verdict) -> Result<(), Self::Error>;

    /// The snapshot everything runs from, once taken.
    fn snapshot(&self) -> Option<&SnapshotId>;
}
