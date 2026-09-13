use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use deterministic_simulator::Runtime;

use crate::{
    connection::Connection,
    roles::{Role, RoleConfig, upstream::MockUpstream},
};

const SV2_PORT: u16 = 34254;

/// The roles under test, running on a seeded simulator.
///
/// Roles are addressed by index, which is what an IR program's `LoadRole` refers to. Each
/// connection the harness opens gets its own address so that frames from different
/// connections never share an inbox.
pub struct Deployment {
    runtime: Runtime,
    roles: Vec<RoleConfig>,
}

impl Deployment {
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

    #[must_use]
    pub fn role_config(&self, role: usize) -> Option<&RoleConfig> {
        self.roles.get(role)
    }

    /// Open a connection from the harness to a role.
    ///
    /// Returns `None` when the connection index is past what the deployment can address.
    #[must_use]
    pub fn connect(&self, connection: usize, role: usize) -> Option<Connection> {
        if connection >= MAX_ADDRESSES || role >= self.roles.len() {
            return None;
        }
        Some(self.connect_unchecked(connection, role))
    }

    fn connect_unchecked(&self, connection: usize, role: usize) -> Connection {
        let handle = self.runtime.local_handle(harness_address(connection));
        Connection::outbound(handle.net.clone(), role_address(role))
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
