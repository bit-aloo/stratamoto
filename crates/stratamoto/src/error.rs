use std::fmt;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Framing(&'static str),
    Framing2(stratum_core::framing_sv2::Error),
    Codec(stratum_core::codec_sv2::Error),
    Parser(stratum_core::parsers_sv2::ParserError),
    Binary(stratum_core::binary_sv2::Error),
    UnexpectedMessage(u8),
    Input(String),
    Noise(String),
    Timeout,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io: {e}"),
            Error::Framing(e) => write!(f, "framing: {e}"),
            Error::Framing2(e) => write!(f, "framing: {e:?}"),
            Error::Codec(e) => write!(f, "codec: {e:?}"),
            Error::Parser(e) => write!(f, "parser: {e:?}"),
            Error::Binary(e) => write!(f, "binary: {e:?}"),
            Error::UnexpectedMessage(t) => write!(f, "unexpected message type 0x{t:02x}"),
            Error::Input(e) => write!(f, "input: {e}"),
            Error::Noise(e) => write!(f, "noise: {e}"),
            Error::Timeout => write!(f, "timed out"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<stratum_core::framing_sv2::Error> for Error {
    fn from(e: stratum_core::framing_sv2::Error) -> Self {
        Error::Framing2(e)
    }
}

impl From<stratum_core::codec_sv2::Error> for Error {
    fn from(e: stratum_core::codec_sv2::Error) -> Self {
        Error::Codec(e)
    }
}

impl From<stratum_core::parsers_sv2::ParserError> for Error {
    fn from(e: stratum_core::parsers_sv2::ParserError) -> Self {
        Error::Parser(e)
    }
}

impl From<stratum_core::binary_sv2::Error> for Error {
    fn from(e: stratum_core::binary_sv2::Error) -> Self {
        Error::Binary(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
