use std::{
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    time::Duration,
};

use stratum_core::{
    codec_sv2::{
        Decoded, Decrypted, ExpectsHandshakeMessage, Handshake, HandshakeMessage, InitiatorSent,
        NoiseDecoder, NoiseEncoder, TransportDecryptState, TransportEncryptState,
    },
    framing_sv2::framing::SerializedFrame,
    noise_sv2::{
        ELLSWIFT_ENCODING_SIZE, INITIATOR_EXPECTED_HANDSHAKE_MESSAGE_SIZE, Initiator, Responder,
    },
};

use crate::{
    connection::{Frame, assemble},
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
    encoder: NoiseEncoder,
    decoder: NoiseDecoder,
    encrypt: TransportEncryptState,
    /// Taken for the duration of a decode, since a failed decryption consumes it and leaves
    /// the connection unusable.
    decrypt: Option<TransportDecryptState>,
    /// The chunk being read, kept across a timed out `recv` so that a frame arriving in pieces
    /// is not torn by a short poll.
    pending: Vec<u8>,
    filled: usize,
}

impl NoiseTransport {
    /// Connect to a role and run the handshake as the initiator, which is what a downstream
    /// does.
    pub fn connect(mut socket: TcpStream, timeout: Duration) -> Result<Self> {
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        let mut encoder = NoiseEncoder::new();
        let mut decoder = NoiseDecoder::new();

        let initiator = Initiator::without_pk().map_err(|e| Error::Noise(format!("{e:?}")))?;

        // The first message is the initiator's ellswift encoded key, sent in the clear: there
        // is no key to encrypt it with yet.
        let (first, handshake) = Handshake::initiator(initiator).step_0()?;
        socket.write_all(encoder.encode_handshake(first).as_ref())?;

        let second = read_handshake::<InitiatorSent>(&mut socket, &mut decoder)?;
        let second: [u8; INITIATOR_EXPECTED_HANDSHAKE_MESSAGE_SIZE] =
            second.payload().try_into().map_err(|_| {
                Error::Noise("the responder sent a malformed handshake message".to_string())
            })?;

        let transport = handshake.step_2(second)?;
        Ok(Self::from_transport(socket, encoder, decoder, transport))
    }

    /// Accept a connection and run the handshake as the responder, which is what an upstream
    /// does. The authority key pair is the one the connecting role expects to see.
    pub fn accept(
        mut socket: TcpStream,
        authority_public_key: [u8; 32],
        authority_private_key: [u8; 32],
        certificate_validity: Duration,
        timeout: Duration,
    ) -> Result<Self> {
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        let mut encoder = NoiseEncoder::new();
        let mut decoder = NoiseDecoder::new();

        let responder = Responder::from_authority_kp(
            &authority_public_key,
            &authority_private_key,
            certificate_validity,
        )
        .map_err(|e| Error::Noise(format!("{e:?}")))?;

        let first = read_handshake::<Responder>(&mut socket, &mut decoder)?;
        let first: [u8; ELLSWIFT_ENCODING_SIZE] = first.payload().try_into().map_err(|_| {
            Error::Noise("the initiator sent a malformed handshake message".to_string())
        })?;

        let (second, transport) = Handshake::responder(responder).step_1(first)?;
        socket.write_all(encoder.encode_handshake(second).as_ref())?;

        Ok(Self::from_transport(socket, encoder, decoder, transport))
    }

    fn from_transport(
        socket: TcpStream,
        encoder: NoiseEncoder,
        decoder: NoiseDecoder,
        transport: stratum_core::codec_sv2::Transport,
    ) -> Self {
        let (encrypt, decrypt) = transport.split();
        Self {
            socket,
            encoder,
            decoder,
            encrypt,
            decrypt: Some(decrypt),
            pending: Vec::new(),
            filled: 0,
        }
    }

    fn read_frame(&mut self) -> Result<Frame> {
        loop {
            // The decoder asks for at most one chunk at a time, so a frame is never read past
            // its end and the next one stays intact in the socket.
            let expected = self.decoder.writable_len();
            if self.pending.len() != expected {
                self.pending.resize(expected, 0);
                self.filled = 0;
            }
            while self.filled < expected {
                match self.socket.read(&mut self.pending[self.filled..]) {
                    Ok(0) => return Err(Error::Io(ErrorKind::UnexpectedEof.into())),
                    Ok(read) => self.filled += read,
                    Err(e) => return Err(Error::Io(e)),
                }
            }
            self.decoder.writable().copy_from_slice(&self.pending);
            self.filled = 0;

            let state = self.decrypt.take().ok_or_else(|| {
                Error::Noise("a decryption failed earlier on this connection".to_string())
            })?;
            match self.decoder.next_transport_frame(state)? {
                Decrypted::Frame(frame, state) => {
                    self.decrypt = Some(state);
                    return Frame::from_bytes(frame.as_bytes().to_vec());
                }
                Decrypted::Incomplete(_, state) => self.decrypt = Some(state),
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
        let frame = SerializedFrame::from_bytes(assemble(
            extension_type,
            message_type,
            channel_msg,
            payload,
        )?)
        .map_err(|_| Error::Framing("could not frame the payload"))?;

        let encoded = self.encoder.encode_transport(frame, &mut self.encrypt)?;
        self.socket.write_all(encoded.as_ref())?;
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Frame> {
        let previous = self.socket.read_timeout()?;
        // A zero duration means no timeout to the socket, so a poll is the shortest wait it
        // accepts.
        self.socket
            .set_read_timeout(Some(timeout.max(Duration::from_millis(1))))?;

        let result = self.read_frame();

        self.socket.set_read_timeout(previous)?;

        match result {
            Err(Error::Io(e))
                if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
                Err(Error::Timeout)
            }
            other => other,
        }
    }
}

/// Read one handshake message, whose length is fixed by the step the role `R` is waiting on.
fn read_handshake<R: ExpectsHandshakeMessage>(
    socket: &mut TcpStream,
    decoder: &mut NoiseDecoder,
) -> Result<HandshakeMessage> {
    loop {
        let mut chunk = vec![0u8; decoder.writable_len()];
        socket.read_exact(&mut chunk)?;
        decoder.writable().copy_from_slice(&chunk);
        if let Decoded::Frame(message) = decoder.next_handshake_frame::<R>()? {
            return Ok(message);
        }
    }
}
