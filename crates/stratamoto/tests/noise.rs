use std::{
    net::{TcpListener, TcpStream},
    thread,
    time::Duration,
};

use stratamoto::{
    noise::NoiseTransport,
    stratum_core::{
        binary_sv2::{GetSize, Serialize, to_writer},
        common_messages_sv2::{
            MESSAGE_TYPE_SETUP_CONNECTION, MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS, Protocol,
            SetupConnection, SetupConnectionSuccess,
        },
        parsers_sv2::{AnyMessage, CommonMessages},
    },
    transport::Transport,
};

/// The key pair sv2-apps' own tests use for the pool's authority.
const AUTHORITY_PUBLIC: &str = "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
const AUTHORITY_SECRET: &str = "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n";

const TIMEOUT: Duration = Duration::from_secs(5);

/// Sv2 encodes a public key as base58check over a two byte little endian key version
/// followed by the 32 byte x only key, and a secret key as base58check over the key alone.
fn public_key(encoded: &str) -> [u8; 32] {
    let decoded = decode(encoded);
    assert_eq!(
        u16::from_le_bytes([decoded[0], decoded[1]]),
        1,
        "key version"
    );
    decoded[2..].try_into().expect("a 32 byte key")
}

fn secret_key(encoded: &str) -> [u8; 32] {
    decode(encoded).try_into().expect("a 32 byte key")
}

fn decode(encoded: &str) -> Vec<u8> {
    bs58::decode(encoded)
        .with_check(None)
        .into_vec()
        .expect("the test keys are valid base58check")
}

fn encode<T: GetSize + Serialize>(message: T) -> Vec<u8> {
    let mut payload = vec![0u8; message.get_size()];
    to_writer(message, &mut payload).expect("the message fits its own size");
    payload
}

fn setup_connection() -> SetupConnection<'static> {
    SetupConnection {
        protocol: Protocol::MiningProtocol,
        min_version: 2,
        max_version: 2,
        flags: 0,
        endpoint_host: "0.0.0.0".try_into().unwrap(),
        endpoint_port: 0,
        vendor: "stratamoto".try_into().unwrap(),
        hardware_version: "".try_into().unwrap(),
        firmware: "".try_into().unwrap(),
        device_id: "".try_into().unwrap(),
    }
}

/// The handshake and the encrypted framing have to interoperate with themselves before there
/// is any hope of them interoperating with a real role.
#[test]
fn an_encrypted_setup_connection_round_trips_over_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("the bound address");

    let upstream = thread::spawn(move || {
        let (socket, _) = listener.accept().expect("the harness connects");
        let mut transport = NoiseTransport::accept(
            socket,
            public_key(AUTHORITY_PUBLIC),
            secret_key(AUTHORITY_SECRET),
            Duration::from_secs(3600),
            TIMEOUT,
        )
        .expect("the responder handshake completes");

        let mut frame = transport.recv(TIMEOUT).expect("a frame arrives");
        let header = frame.header();
        assert_eq!(header.msg_type(), MESSAGE_TYPE_SETUP_CONNECTION);

        match frame.message().expect("the payload decodes") {
            AnyMessage::Common(CommonMessages::SetupConnection(setup)) => {
                assert_eq!(setup.protocol, Protocol::MiningProtocol);
                assert_eq!(setup.max_version, 2);
            }
            other => panic!("expected a SetupConnection, got {other}"),
        }

        transport
            .send(
                0,
                MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
                false,
                &encode(SetupConnectionSuccess {
                    used_version: 2,
                    flags: 0,
                }),
            )
            .expect("the answer is sent");
    });

    let socket = TcpStream::connect(address).expect("the upstream is listening");
    let mut transport =
        NoiseTransport::connect(socket, TIMEOUT).expect("the initiator handshake completes");

    transport
        .send(
            0,
            MESSAGE_TYPE_SETUP_CONNECTION,
            false,
            &encode(setup_connection()),
        )
        .expect("the setup is sent");

    let mut frame = transport.recv(TIMEOUT).expect("the answer arrives");
    match frame.message().expect("the answer decodes") {
        AnyMessage::Common(CommonMessages::SetupConnectionSuccess(success)) => {
            assert_eq!(success.used_version, 2);
        }
        other => panic!("expected a SetupConnection.Success, got {other}"),
    }

    upstream.join().expect("the upstream thread does not panic");
}

/// Nothing readable may cross the wire in the clear once the handshake is done.
#[test]
fn the_payload_is_not_sent_in_the_clear() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
    let address = listener.local_addr().expect("the bound address");

    let observer = thread::spawn(move || {
        use std::io::Read;
        let (mut socket, _) = listener.accept().expect("the harness connects");

        // Complete the handshake by hand so that the raw bytes after it can be inspected.
        let mut initiator_key = [0u8; 64];
        socket
            .read_exact(&mut initiator_key)
            .expect("the first message");

        let mut responder = stratamoto::stratum_core::noise_sv2::Responder::from_authority_kp(
            &public_key(AUTHORITY_PUBLIC),
            &secret_key(AUTHORITY_SECRET),
            Duration::from_secs(3600),
        )
        .expect("valid keys");
        let response = responder
            .step_1(initiator_key)
            .expect("the responder step succeeds");
        use std::io::Write;
        socket.write_all(&response.0).expect("the second message");

        let mut seen = vec![0u8; 512];
        let read = socket.read(&mut seen).expect("the setup arrives");
        seen.truncate(read);
        seen
    });

    let socket = TcpStream::connect(address).expect("the upstream is listening");
    let mut transport =
        NoiseTransport::connect(socket, TIMEOUT).expect("the initiator handshake completes");
    transport
        .send(
            0,
            MESSAGE_TYPE_SETUP_CONNECTION,
            false,
            &encode(setup_connection()),
        )
        .expect("the setup is sent");

    let seen = observer.join().expect("the observer thread does not panic");
    assert!(
        !seen.windows(10).any(|w| w == b"stratamoto"),
        "the vendor string crossed the wire in the clear"
    );
}
