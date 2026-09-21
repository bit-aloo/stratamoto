use stratum_apps::stratum_core::{
    framing_sv2::{framing::SerializedFrame, header::Header},
    parsers_sv2::AnyMessage,
};

use crate::error::{Error, Result};

const CHANNEL_MSG_MASK: u16 = 0b1000_0000_0000_0000;

/// A received frame, owning the bytes its message borrows from.
pub struct Frame(SerializedFrame<Vec<u8>>);

impl Frame {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        SerializedFrame::from_bytes(bytes)
            .map(Frame)
            .map_err(|_| Error::Framing("bytes do not hold exactly one frame"))
    }

    pub fn header(&self) -> Header {
        self.0.header()
    }

    pub fn message(&mut self) -> Result<AnyMessage<'_>> {
        let header = self.0.header();
        Ok((header, self.0.payload()).try_into()?)
    }
}

/// Lay out a frame: the six byte header, then the payload.
///
/// The header is written here rather than delegated, so that a program can put anything it
/// likes on the wire.
pub fn assemble(
    extension_type: u16,
    message_type: u8,
    channel_msg: bool,
    payload: &[u8],
) -> Result<Vec<u8>> {
    let length = u32::try_from(payload.len())
        .ok()
        .filter(|len| *len < 1 << 24)
        .ok_or(Error::Framing("payload exceeds the 24 bit length field"))?;

    let extension_type = if channel_msg {
        extension_type | CHANNEL_MSG_MASK
    } else {
        extension_type & !CHANNEL_MSG_MASK
    };

    let mut bytes = Vec::with_capacity(Header::SIZE + payload.len());
    bytes.extend_from_slice(&extension_type.to_le_bytes());
    bytes.push(message_type);
    bytes.extend_from_slice(&length.to_le_bytes()[..3]);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}
