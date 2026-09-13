use serde::{Deserialize, Serialize};

/// The subprotocol a connection is set up for.
///
/// Mirrors the `protocol` field of `SetupConnection`, but is kept independent of the
/// wire types so that the IR stays serializable and implementation agnostic.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Protocol {
    Mining,
    JobDeclaration,
    TemplateDistribution,
}

impl Protocol {
    #[must_use]
    pub fn id(&self) -> u8 {
        match self {
            Protocol::Mining => 0,
            Protocol::JobDeclaration => 1,
            Protocol::TemplateDistribution => 2,
        }
    }

    #[must_use]
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Protocol::Mining),
            1 => Some(Protocol::JobDeclaration),
            2 => Some(Protocol::TemplateDistribution),
            _ => None,
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::Mining => write!(f, "mining"),
            Protocol::JobDeclaration => write!(f, "job-declaration"),
            Protocol::TemplateDistribution => write!(f, "template-distribution"),
        }
    }
}

/// `Variable` is the type of a value produced by an [`crate::Operation`].
///
/// Operations declare the variable types they consume, so a program can only wire a value
/// into a position that accepts it. Protocol carrying types are what tie a subprotocol's
/// messages to the connection they may be sent on.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Hash)]
pub enum Variable {
    /// Output type of no-op instructions.
    Nop,

    /// Index of a role in the deployment.
    Role,
    /// A transport between two roles, before any Sv2 message is exchanged.
    Connection,
    /// A connection that has been set up for a subprotocol.
    Session(Protocol),

    Version,
    Flags,
    Port,
    Str,
    Bytes,
    Duration,

    /// A `SetupConnection` under construction.
    MutSetupConnection,
    /// A finalized `SetupConnection` for a subprotocol.
    ConstSetupConnection(Protocol),
}

impl std::fmt::Display for Variable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Variable::Nop => write!(f, "nop"),
            Variable::Role => write!(f, "role"),
            Variable::Connection => write!(f, "connection"),
            Variable::Session(p) => write!(f, "session<{p}>"),
            Variable::Version => write!(f, "version"),
            Variable::Flags => write!(f, "flags"),
            Variable::Port => write!(f, "port"),
            Variable::Str => write!(f, "str"),
            Variable::Bytes => write!(f, "bytes"),
            Variable::Duration => write!(f, "duration"),
            Variable::MutSetupConnection => write!(f, "mut-setup-connection"),
            Variable::ConstSetupConnection(p) => write!(f, "setup-connection<{p}>"),
        }
    }
}
