use std::{
    io::ErrorKind,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use stratamoto::{
    error::Error as TransportError,
    noise::NoiseTransport,
    stratum_core::{
        binary_sv2::{GetSize, Serialize, U256, to_writer},
        common_messages_sv2::{
            MESSAGE_TYPE_SETUP_CONNECTION, MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
            SetupConnectionSuccess,
        },
        template_distribution_sv2::{
            MESSAGE_TYPE_NEW_TEMPLATE, MESSAGE_TYPE_SET_NEW_PREV_HASH, NewTemplate,
            SetNewPrevHash,
        },
    },
    transport::Transport,
};

use crate::Error;

/// The authority key pair sv2-apps' own tests use, raw and base58check encoded. A role that
/// connects without a key to verify, as the pool does to its Template Provider, accepts any
/// valid pair.
pub(crate) const AUTHORITY_PUBLIC_KEY: [u8; 32] = [
    36, 238, 60, 56, 4, 161, 170, 164, 192, 59, 128, 234, 25, 247, 165, 134, 60, 145, 110, 137,
    148, 183, 219, 148, 163, 186, 215, 238, 9, 43, 108, 231,
];
pub(crate) const AUTHORITY_SECRET_KEY: [u8; 32] = [
    101, 153, 94, 177, 150, 49, 244, 120, 164, 111, 250, 92, 241, 229, 69, 9, 30, 254, 149, 14,
    174, 172, 116, 130, 255, 220, 6, 235, 106, 137, 246, 151,
];
pub(crate) const AUTHORITY_PUBLIC_KEY_ENCODED: &str =
    "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
pub(crate) const AUTHORITY_SECRET_KEY_ENCODED: &str =
    "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n";

const IO_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);
const CERTIFICATE_VALIDITY: Duration = Duration::from_secs(3600);

const TEMPLATE_ID: u64 = 1;

/// The future template channels-sv2's own tests build: a coinbase whose only output is the
/// witness commitment.
static COINBASE_PREFIX: [u8; 2] = [82, 0];
static COINBASE_OUTPUTS: [u8; 47] = [
    0, 0, 0, 0, 0, 0, 0, 0, 38, 106, 36, 170, 33, 169, 237, 226, 246, 28, 63, 113, 209, 222, 253,
    63, 169, 153, 223, 163, 105, 83, 117, 92, 105, 6, 137, 121, 153, 98, 180, 139, 235, 216, 54,
    151, 78, 140, 249,
];
static PREV_HASH: [u8; 32] = [0; 32];
/// The regtest limit, `0x207fffff` expanded, as a little endian `U256`.
static TARGET: [u8; 32] = {
    let mut target = [0u8; 32];
    target[29] = 0xff;
    target[30] = 0xff;
    target[31] = 0x7f;
    target
};
const N_BITS: u32 = 0x207f_ffff;
const HEADER_TIMESTAMP: u32 = 1_700_000_000;

/// A Template Provider the harness plays for a real role.
///
/// It answers the role's `SetupConnection`, then hands it one future template and the
/// `SetNewPrevHash` that activates it, which is everything a pool waits for before it accepts
/// downstreams. After that it only drains what the role sends, so the connection stays open and
/// the role never blocks writing to it.
pub struct TemplateProvider {
    address: SocketAddr,
    served: Receiver<Result<(), String>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl TemplateProvider {
    pub fn start() -> Result<Self, Error> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        // Accept polls, so that a role that never connects cannot keep the thread from
        // stopping.
        listener.set_nonblocking(true)?;

        let (report, served) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();

        let thread = thread::spawn(move || {
            let transport = match provide(&listener, &stopping) {
                Ok(transport) => {
                    let _ = report.send(Ok(()));
                    transport
                }
                Err(e) => {
                    let _ = report.send(Err(e.to_string()));
                    return;
                }
            };
            drain(transport, &stopping);
        });

        Ok(Self {
            address,
            served,
            stop,
            thread: Some(thread),
        })
    }

    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Wait until the role has been handed its template, or report why it was not.
    pub fn wait_until_served(&self, timeout: Duration) -> Result<(), Error> {
        match self.served.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(reason)) => Err(Error::Startup(reason)),
            Err(_) => Err(Error::Startup(
                "the role never connected to the template provider".to_string(),
            )),
        }
    }
}

impl Drop for TemplateProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn provide(listener: &TcpListener, stop: &AtomicBool) -> Result<NoiseTransport, Error> {
    let socket = accept(listener, stop)?;
    let mut transport = NoiseTransport::accept(
        socket,
        AUTHORITY_PUBLIC_KEY,
        AUTHORITY_SECRET_KEY,
        CERTIFICATE_VALIDITY,
        IO_TIMEOUT,
    )?;

    let frame = transport.recv(IO_TIMEOUT)?;
    if frame.header().msg_type() != MESSAGE_TYPE_SETUP_CONNECTION {
        return Err(Error::Startup(format!(
            "the role opened with message type 0x{:02x}, not SetupConnection",
            frame.header().msg_type()
        )));
    }

    transport.send(
        0,
        MESSAGE_TYPE_SETUP_CONNECTION_SUCCESS,
        false,
        &encode(SetupConnectionSuccess {
            used_version: 2,
            flags: 0,
        })?,
    )?;

    let template = NewTemplate {
        template_id: TEMPLATE_ID,
        future_template: true,
        version: 0x2000_0000,
        coinbase_tx_version: 2,
        coinbase_prefix: (&COINBASE_PREFIX[..]).try_into()?,
        coinbase_tx_input_sequence: u32::MAX,
        coinbase_tx_value_remaining: 5_000_000_000,
        coinbase_tx_outputs_count: 1,
        coinbase_tx_outputs: (&COINBASE_OUTPUTS[..]).try_into()?,
        coinbase_tx_locktime: 0,
        merkle_path: Vec::<U256>::new().try_into()?,
    };
    transport.send(0, MESSAGE_TYPE_NEW_TEMPLATE, false, &encode(template)?)?;

    let prev_hash = SetNewPrevHash {
        template_id: TEMPLATE_ID,
        prev_hash: (&PREV_HASH).into(),
        header_timestamp: HEADER_TIMESTAMP,
        n_bits: N_BITS,
        target: (&TARGET).into(),
    };
    transport.send(0, MESSAGE_TYPE_SET_NEW_PREV_HASH, false, &encode(prev_hash)?)?;

    Ok(transport)
}

fn accept(listener: &TcpListener, stop: &AtomicBool) -> Result<TcpStream, Error> {
    loop {
        if stop.load(Ordering::SeqCst) {
            return Err(Error::Startup(
                "stopped before the role connected".to_string(),
            ));
        }
        match listener.accept() {
            Ok((socket, _)) => {
                socket.set_nonblocking(false)?;
                return Ok(socket);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(e) => return Err(e.into()),
        }
    }
}

fn drain(mut transport: NoiseTransport, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match transport.recv(POLL) {
            Ok(_) | Err(TransportError::Timeout) => {}
            Err(e) => {
                log::debug!("template provider connection closed: {e}");
                return;
            }
        }
    }
}

fn encode<T: Serialize + GetSize>(message: T) -> Result<Vec<u8>, Error> {
    let mut payload = vec![0u8; message.get_size()];
    to_writer(message, &mut payload)?;
    Ok(payload)
}
