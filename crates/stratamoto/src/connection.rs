use std::{net::SocketAddr, time::Duration};

use deterministic_simulator::net::NetworkLocalHandle;
use stratum_core::{
    framing_sv2::{framing::Sv2Frame, header::Header},
    parsers_sv2::{AnyMessage, IsSv2Message},
};

use crate::error::{Error, Result};

const TAG: u64 = 0;
const EXTENSION_TYPE: u16 = 0;
const CHANNEL_MSG_MASK: u16 = 0b1000_0000_0000_0000;
const HEADER_SIZE: usize = 6;

/// A received frame, owning the bytes its message borrows from.
pub struct Frame(Sv2Frame<AnyMessage<'static>, Vec<u8>>);

impl Frame {
    pub fn header(&self) -> Result<Header> {
        self.0.get_header().ok_or(Error::Framing("missing header"))
    }

    pub fn message(&mut self) -> Result<AnyMessage<'_>> {
        let header = self.header()?;
        Ok((header, self.0.payload()).try_into()?)
    }
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
    ///
    /// The 6 byte header is written here rather than delegated, so that a program can put
    /// anything it likes on the wire.
    pub async fn send_frame(
        &self,
        extension_type: u16,
        message_type: u8,
        channel_msg: bool,
        payload: &[u8],
    ) -> Result<()> {
        let length = u32::try_from(payload.len())
            .ok()
            .filter(|len| *len < 1 << 24)
            .ok_or(Error::Framing("payload exceeds the 24 bit length field"))?;

        let extension_type = if channel_msg {
            extension_type | CHANNEL_MSG_MASK
        } else {
            extension_type & !CHANNEL_MSG_MASK
        };

        let mut bytes = Vec::with_capacity(HEADER_SIZE + payload.len());
        bytes.extend_from_slice(&extension_type.to_le_bytes());
        bytes.push(message_type);
        bytes.extend_from_slice(&length.to_le_bytes()[..3]);
        bytes.extend_from_slice(payload);

        let peer = self.peer.ok_or(Error::Framing("connection has no peer"))?;
        self.net.send_to(peer, TAG, &bytes).await?;
        Ok(())
    }

    pub async fn send(&self, message: AnyMessage<'_>) -> Result<()> {
        let message_type = message.message_type();
        let channel_bit = message.channel_bit();
        let frame = Sv2Frame::<AnyMessage, Vec<u8>>::from_message(
            message,
            message_type,
            EXTENSION_TYPE,
            channel_bit,
        )
        .ok_or(Error::Framing("message does not fit in a frame"))?;

        let mut bytes = vec![0u8; frame.encoded_length()];
        frame.serialize(&mut bytes)?;

        let peer = self.peer.ok_or(Error::Framing("connection has no peer"))?;
        self.net.send_to(peer, TAG, &bytes).await?;
        Ok(())
    }

    /// Take a frame that has already been delivered, without waiting for one.
    pub async fn try_recv(&mut self) -> Option<Frame> {
        let (bytes, _, from) = self.net.try_recv()?;
        self.peer = Some(from);
        Sv2Frame::from_bytes(bytes.to_vec()).ok().map(Frame)
    }

    /// Await the next frame, or give up after `timeout`.
    pub async fn recv_timeout(&mut self, timeout: Duration) -> Result<Frame> {
        match deterministic_simulator::time::timeout(timeout, self.net.recv()).await {
            Ok(received) => {
                let (bytes, _, from) = received?;
                self.peer = Some(from);
                Ok(Frame(
                    Sv2Frame::from_bytes(bytes.to_vec())
                        .map_err(|_| Error::Framing("incomplete frame"))?,
                ))
            }
            Err(_) => Err(Error::Timeout),
        }
    }

    pub async fn recv(&mut self) -> Result<Frame> {
        let (bytes, _, from) = self.net.recv().await?;
        self.peer = Some(from);
        let frame = Sv2Frame::from_bytes(bytes.to_vec())
            .map_err(|_| Error::Framing("incomplete frame"))?;
        Ok(Frame(frame))
    }
}
