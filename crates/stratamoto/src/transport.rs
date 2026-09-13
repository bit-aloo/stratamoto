use std::time::Duration;

use crate::{connection::Frame, error::Result};

/// A link the harness can put Sv2 frames on.
///
/// The interface is synchronous because a real role runs on its own runtime behind a real
/// socket, while a simulated one runs on the deterministic executor; a blocking call is the
/// only shape both can offer. The simulated implementation drives its executor for the
/// duration of the call, so time only advances while the harness is waiting.
pub trait Transport {
    fn send(
        &mut self,
        extension_type: u16,
        message_type: u8,
        channel_msg: bool,
        payload: &[u8],
    ) -> Result<()>;

    /// Await one frame, giving up after `timeout`.
    fn recv(&mut self, timeout: Duration) -> Result<Frame>;
}

/// The roles a program can be run against.
pub trait Deployment {
    type Transport<'a>: Transport
    where
        Self: 'a;

    /// Open a link to a role, or `None` if the deployment cannot address it.
    fn connect(&self, connection: usize, role: usize) -> Option<Self::Transport<'_>>;

    fn role_config(&self, role: usize) -> Option<&crate::roles::RoleConfig>;

    fn num_roles(&self) -> usize;

    /// Let the deployment run for a while without the harness doing anything.
    fn advance_time(&self, duration: std::time::Duration);
}
