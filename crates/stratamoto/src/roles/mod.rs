pub mod upstream;

use stratum_apps::stratum_core::common_messages_sv2::Protocol;

use crate::{connection::Connection, error::Result};

/// A component of an Sv2 deployment, driven by the simulator as a spawned task.
pub trait Role {
    fn run(self, connection: Connection) -> impl Future<Output = Result<()>> + Send;
}

/// What a role accepts on a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoleConfig {
    pub protocol: Protocol,
    pub min_version: u16,
    pub max_version: u16,
    pub supported_flags: u32,
}

impl RoleConfig {
    #[must_use]
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            min_version: 2,
            max_version: 2,
            supported_flags: 0,
        }
    }

    #[must_use]
    pub fn with_versions(mut self, min_version: u16, max_version: u16) -> Self {
        self.min_version = min_version;
        self.max_version = max_version;
        self
    }

    #[must_use]
    pub fn with_supported_flags(mut self, supported_flags: u32) -> Self {
        self.supported_flags = supported_flags;
        self
    }
}

/// The subprotocol a role serves, as the wire type spells it.
#[must_use]
pub fn protocol_of(protocol: stratamoto_ir::Protocol) -> Protocol {
    match protocol {
        stratamoto_ir::Protocol::Mining => Protocol::MiningProtocol,
        stratamoto_ir::Protocol::JobDeclaration => Protocol::JobDeclarationProtocol,
        stratamoto_ir::Protocol::TemplateDistribution => Protocol::TemplateDistributionProtocol,
    }
}
