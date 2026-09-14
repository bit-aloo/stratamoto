pub mod pool;
pub mod template_provider;

use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Transport(stratamoto::error::Error),
    Encoding(String),
    /// The role under test could not be brought to the point of accepting connections.
    Startup(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Transport(e) => write!(f, "transport: {e}"),
            Error::Encoding(e) => write!(f, "encoding: {e}"),
            Error::Startup(e) => write!(f, "startup: {e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<stratamoto::error::Error> for Error {
    fn from(e: stratamoto::error::Error) -> Self {
        Error::Transport(e)
    }
}

impl From<stratamoto::stratum_core::binary_sv2::Error> for Error {
    fn from(e: stratamoto::stratum_core::binary_sv2::Error) -> Self {
        Error::Encoding(format!("{e:?}"))
    }
}
