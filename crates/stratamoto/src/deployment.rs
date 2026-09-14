use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use deterministic_simulator::Runtime;

use crate::{
    connection::{Connection, Frame},
    error::{Error, Result},
    roles::{Role, RoleConfig, upstream::MockUpstream},
    transport::{Deployment as DeploymentTrait, Transport},
};

const SV2_PORT: u16 = 34254;

/// The roles under test, running on a seeded simulator.
///
/// Roles are addressed by index, which is what an IR program's `LoadRole` refers to. Each
/// connection the harness opens gets its own address so that frames from different
/// connections never share an inbox.
pub struct SimulatedDeployment {
    runtime: Runtime,
    roles: Vec<RoleConfig>,
}

impl SimulatedDeployment {
    #[must_use]
    pub fn new(seed: u64, roles: Vec<RoleConfig>) -> Self {
        let runtime = Runtime::new_with_seed(seed);

        for (index, config) in roles.iter().copied().enumerate() {
            let handle = runtime.local_handle(role_address(index));
            let connection = Connection::inbound(handle.net.clone());
            handle
                .spawn(async move {
                    if let Err(e) = MockUpstream::new(config).run(connection).await {
                        log::debug!("role stopped: {e}");
                    }
                })
                .detach();
        }

        Self { runtime, roles }
    }

    #[must_use]
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    #[must_use]
    pub fn roles(&self) -> &[RoleConfig] {
        &self.roles
    }
}

impl DeploymentTrait for SimulatedDeployment {
    type Transport<'a> = SimulatedTransport<'a>;

    fn connect(&self, connection: usize, role: usize) -> Option<SimulatedTransport<'_>> {
        if connection >= MAX_ADDRESSES || role >= self.roles.len() {
            return None;
        }
        let handle = self.runtime.local_handle(harness_address(connection));
        Some(SimulatedTransport {
            runtime: &self.runtime,
            connection: Connection::outbound(handle.net.clone(), role_address(role)),
        })
    }

    fn role_config(&self, role: usize) -> Option<&RoleConfig> {
        self.roles.get(role)
    }

    fn num_roles(&self) -> usize {
        self.roles.len()
    }

    fn advance_time(&self, duration: Duration) {
        self.runtime
            .block_on(deterministic_simulator::time::sleep(duration));
    }
}

/// A link to a role running on the simulator.
///
/// Each call drives the executor until it completes, which is what keeps the roles making
/// progress between the harness's own steps.
pub struct SimulatedTransport<'a> {
    runtime: &'a Runtime,
    connection: Connection,
}

impl Transport for SimulatedTransport<'_> {
    fn send(
        &mut self,
        extension_type: u16,
        message_type: u8,
        channel_msg: bool,
        payload: &[u8],
    ) -> Result<()> {
        self.runtime.block_on(self.connection.send_frame(
            extension_type,
            message_type,
            channel_msg,
            payload,
        ))
    }

    fn recv(&mut self, timeout: Duration) -> Result<Frame> {
        if timeout.is_zero() {
            // The executor panics when asked to advance with nothing pending, so a poll for
            // an already delivered frame cannot go through the timeout path.
            return self
                .runtime
                .block_on(self.connection.try_recv())
                .ok_or(Error::Timeout);
        }
        self.runtime.block_on(self.connection.recv_timeout(timeout))
    }
}

/// The most roles or connections a deployment can address.
///
/// Each gets its own address out of a /16, so that frames from different connections never
/// share an inbox.
pub const MAX_ADDRESSES: usize = u16::MAX as usize;

fn role_address(role: usize) -> SocketAddr {
    address(10, 0, role)
}

fn harness_address(connection: usize) -> SocketAddr {
    address(10, 1, connection)
}

fn address(a: u8, b: u8, index: usize) -> SocketAddr {
    assert!(index < MAX_ADDRESSES, "index {index} exceeds MAX_ADDRESSES");
    let index = index as u16 + 1;
    let [high, low] = index.to_be_bytes();
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(a, b, high, low)), SV2_PORT)
}
