use std::{
    io::{Read, Write},
    net::TcpStream,
    time::Duration,
};

use stratum_core::{
    codec_sv2::{Error as CodecError, HandshakeRole, NoiseEncoder, StandardNoiseDecoder, State},
    framing_sv2::framing::{Frame as CodecFrame, Sv2Frame},
    noise_sv2::{
        ELLSWIFT_ENCODING_SIZE, INITIATOR_EXPECTED_HANDSHAKE_MESSAGE_SIZE, Initiator, Responder,
    },
    parsers_sv2::AnyMessage,
};

use crate::{
    connection::Frame,
    error::{Error, Result},
    transport::Transport,
};

/// Sv2 connections between real roles are always Noise encrypted, so a harness that talks to
/// one has to complete the handshake before any message is exchanged.
///
/// The socket is blocking because the harness is: a real role runs on its own runtime, and
/// there is nothing for the harness to do while it waits.
pub struct NoiseTransport {
    socket: TcpStream,
    state: State,
    encoder: NoiseEncoder<AnyMessage<'static>>,
    decoder: StandardNoiseDecoder<AnyMessage<'static>>,
}

impl NoiseTransport {
    /// Connect to a role and run the handshake as the initiator, which is what a downstream
    /// does.
    pub fn connect(socket: TcpStream, timeout: Duration) -> Result<Self> {
        let initiator = Initiator::without_pk().map_err(|e| Error::Noise(format!("{e:?}")))?;
        let mut state = State::initialized(HandshakeRole::Initiator(initiator));

        socket
            .set_read_timeout(Some(timeout))
            .and_then(|()| socket.set_write_timeout(Some(timeout)))
            .map_err(Error::Io)?;
        let mut socket = socket;

        // The first handshake message is the initiator's ellswift encoded public key, sent as
        // plain bytes: there is no key to encrypt it with yet.
        let first = state.step_0().map_err(Error::Codec)?;
        socket
            .write_all(&first.get_payload_when_handshaking())
            .map_err(Error::Io)?;

        let mut response = [0u8; INITIATOR_EXPECTED_HANDSHAKE_MESSAGE_SIZE];
        socket.read_exact(&mut response).map_err(Error::Io)?;

        let state = state.step_2(response).map_err(Error::Codec)?;
        Ok(Self::from_state(socket, state))
    }

    /// Accept a connection and run the handshake as the responder, which is what an upstream
    /// does. The authority key pair is the one the connecting role expects to see.
    pub fn accept(
        socket: TcpStream,
        authority_public_key: [u8; 32],
        authority_private_key: [u8; 32],
        certificate_validity: Duration,
        timeout: Duration,
    ) -> Result<Self> {
        let responder = Responder::from_authority_kp(
            &authority_public_key,
            &authority_private_key,
            certificate_validity,
        )
        .map_err(|e| Error::Noise(format!("{e:?}")))?;
        let mut state = State::initialized(HandshakeRole::Responder(responder));

        socket
            .set_read_timeout(Some(timeout))
            .and_then(|()| socket.set_write_timeout(Some(timeout)))
            .map_err(Error::Io)?;
        let mut socket = socket;

        let mut initiator_key = [0u8; ELLSWIFT_ENCODING_SIZE];
        socket.read_exact(&mut initiator_key).map_err(Error::Io)?;

        let (response, state) = state.step_1(initiator_key).map_err(Error::Codec)?;
        socket
            .write_all(&response.get_payload_when_handshaking())
            .map_err(Error::Io)?;

        Ok(Self::from_state(socket, state))
    }

    fn from_state(socket: TcpStream, state: State) -> Self {
        Self {
            socket,
            state,
            encoder: NoiseEncoder::new(),
            decoder: StandardNoiseDecoder::new(),
        }
    }

    fn read_frame(&mut self) -> Result<Frame> {
        loop {
            // The decoder asks for exactly the bytes it is missing, so a frame is never read
            // past its end and the next one stays intact in the socket.
            match self.decoder.next_frame(&mut self.state) {
                Ok(frame) => return into_frame(frame),
                Err(CodecError::MissingBytes(_)) => {
                    let writable = self.decoder.writable();
                    self.socket.read_exact(writable).map_err(Error::Io)?;
                }
                Err(e) => return Err(Error::Codec(e)),
            }
        }
    }
}

impl Transport for NoiseTransport {
    fn send(
        &mut self,
        extension_type: u16,
        message_type: u8,
        channel_msg: bool,
        payload: &[u8],
    ) -> Result<()> {
        let frame: Sv2Frame<AnyMessage<'static>, Vec<u8>> =
            Sv2Frame::from_bytes(assemble(extension_type, message_type, channel_msg, payload)?)
                .map_err(|_| Error::Framing("could not frame the payload"))?;

        let encoded = self
            .encoder
            .encode(frame.into(), &mut self.state)
            .map_err(Error::Codec)?;
        self.socket.write_all(encoded.as_ref()).map_err(Error::Io)?;
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Frame> {
        let previous = self.socket.read_timeout().map_err(Error::Io)?;
        self.socket
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))
            .map_err(Error::Io)?;

        let result = self.read_frame();

        self.socket.set_read_timeout(previous).map_err(Error::Io)?;

        match result {
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Err(Error::Timeout)
            }
            other => other,
        }
    }
}

/// Build the six byte header and put the payload behind it.
fn assemble(
    extension_type: u16,
    message_type: u8,
    channel_msg: bool,
    payload: &[u8],
) -> Result<Vec<u8>> {
    const CHANNEL_MSG_MASK: u16 = 0b1000_0000_0000_0000;

    let length = u32::try_from(payload.len())
        .ok()
        .filter(|len| *len < 1 << 24)
        .ok_or(Error::Framing("payload exceeds the 24 bit length field"))?;

    let extension_type = if channel_msg {
        extension_type | CHANNEL_MSG_MASK
    } else {
        extension_type & !CHANNEL_MSG_MASK
    };

    let mut bytes = Vec::with_capacity(6 + payload.len());
    bytes.extend_from_slice(&extension_type.to_le_bytes());
    bytes.push(message_type);
    bytes.extend_from_slice(&length.to_le_bytes()[..3]);
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Re-assemble a decoded frame into the owned form the rest of the harness uses.
fn into_frame<B: AsRef<[u8]> + AsMut<[u8]>>(
    frame: CodecFrame<AnyMessage<'static>, B>,
) -> Result<Frame> {
    let mut frame: Sv2Frame<AnyMessage<'static>, B> = frame
        .try_into()
        .map_err(|_| Error::Framing("expected an Sv2 frame, got a handshake frame"))?;

    let header = frame
        .get_header()
        .ok_or(Error::Framing("decoded frame has no header"))?;

    let payload = frame.payload().to_vec();
    let bytes = assemble(
        header.ext_type_without_channel_msg(),
        header.msg_type(),
        header.channel_msg(),
        &payload,
    )?;

    Sv2Frame::from_bytes(bytes)
        .map(Frame)
        .map_err(|_| Error::Framing("could not re-frame a decoded message"))
}
