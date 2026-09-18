use std::{net::SocketAddr, time::Duration};

use stratamoto_dst::net::NetworkLocalHandle;
use stratum_core::{
    framing_sv2::{
        framing::{EncodableFrame, MessageFrame, SerializedFrame},
        header::Header,
    },
    parsers_sv2::{AnyMessage, IsSv2Message},
};

use crate::error::{Error, Result};

const TAG: u64 = 0;
const EXTENSION_TYPE: u16 = 0;
const CHANNEL_MSG_MASK: u16 = 0b1000_0000_0000_0000;

/// A received frame, owning the bytes its message borrows from.
pub struct Frame(SerializedFrame<Vec<u8>>);

impl Frame {
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
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
pub(crate) fn assemble(
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

/// A point to point Sv2 link between two addresses of the simulated network.
///
/// One frame is carried per network message, so framing is never split or coalesced.
pub struct Connection {
    net: NetworkLocalHandle,
    peer: Option<SocketAddr>,
}

impl Connection {
    pub fn outbound(net: NetworkLocalHandle, peer: SocketAddr) -> Self {
        Self {
            net,
            peer: Some(peer),
        }
    }

    pub fn inbound(net: NetworkLocalHandle) -> Self {
        Self { net, peer: None }
    }

    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer
    }

    /// Send a frame with a payload the caller has already encoded.
    pub async fn send_frame(
        &self,
        extension_type: u16,
        message_type: u8,
        channel_msg: bool,
        payload: &[u8],
    ) -> Result<()> {
        let bytes = assemble(extension_type, message_type, channel_msg, payload)?;
        let peer = self.peer.ok_or(Error::Framing("connection has no peer"))?;
        self.net.send_to(peer, TAG, &bytes).await?;
        Ok(())
    }

    pub async fn send(&self, message: AnyMessage<'_>) -> Result<()> {
        let message_type = message.message_type();
        let channel_bit = message.channel_bit();
        let frame = MessageFrame::from_message(message, message_type, EXTENSION_TYPE, channel_bit)?;

        let mut bytes = vec![0u8; frame.encoded_length()];
        frame.encode_into(&mut bytes)?;

        let peer = self.peer.ok_or(Error::Framing("connection has no peer"))?;
        self.net.send_to(peer, TAG, &bytes).await?;
        Ok(())
    }

    /// Take a frame that has already been delivered, without waiting for one.
    pub async fn try_recv(&mut self) -> Option<Frame> {
        let (bytes, _, from) = self.net.try_recv()?;
        self.peer = Some(from);
        Frame::from_bytes(bytes.to_vec()).ok()
    }

    /// Await the next frame, or give up after `timeout`.
    pub async fn recv_timeout(&mut self, timeout: Duration) -> Result<Frame> {
        match stratamoto_dst::time::timeout(timeout, self.net.recv()).await {
            Ok(received) => {
                let (bytes, _, from) = received?;
                self.peer = Some(from);
                Frame::from_bytes(bytes.to_vec())
            }
            Err(_) => Err(Error::Timeout),
        }
    }

    pub async fn recv(&mut self) -> Result<Frame> {
        let (bytes, _, from) = self.net.recv().await?;
        self.peer = Some(from);
        Frame::from_bytes(bytes.to_vec())
    }
}
