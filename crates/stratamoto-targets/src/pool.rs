use std::{
    fs::File,
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use stratamoto::{
    noise::NoiseTransport, roles::RoleConfig, stratum_core::common_messages_sv2::Protocol,
    transport::Deployment,
};

use crate::{
    Error,
    node::{AUTHORITY_PUBLIC_KEY_ENCODED, AUTHORITY_SECRET_KEY_ENCODED, IPC_VERSION, Node},
};

/// The environment variable naming the pool binary, when no path is given.
pub const POOL_BINARY_ENV: &str = "STRATAMOTO_POOL";

const COINBASE_REWARD_DESCRIPTOR: &str = "addr(tb1qa0sm0hxzj0x25rh8gw5xlzwlsfvvyz8u96w3p8)";
const SHARES_PER_MINUTE: f32 = 120.0;
const CERTIFICATE_VALIDITY_SECS: u64 = 3600;
/// The least time between two templates drawn by rising fees, in seconds.
const TEMPLATE_INTERVAL_SECS: u8 = 1;

/// The pool looks for its first template once a second, so readiness takes at least that long.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// A pool that is up answers a handshake immediately, so a liveness probe waits only briefly.
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a connection to an already running pool is retried through transient starvation.
const CONNECT_RETRY_BUDGET: Duration = Duration::from_secs(3);

/// sv2-apps' pool, running as its own process against a Bitcoin Core node it reaches over IPC.
///
/// The pool is the binary sv2-apps builds, started with a configuration written for it, the
/// way a snapshotting fuzzer runs a target: as a separate process whose crashes are the
/// process's own, and whose coverage instrumentation reads its environment at start. Every
/// connection a program opens is a fresh TCP connection with its own handshake. The pool
/// remembers what earlier connections did, so what keeps runs apart is running one program per
/// pool, or restoring a snapshot taken before the first.
pub struct PoolDeployment {
    // Declared before the node so that it is dropped first.
    pool: Pool,
    roles: Vec<RoleConfig>,
    node: Node,
}

/// The pool process, the address it serves on, and the directory its configuration and log
/// live in.
struct Pool {
    child: Child,
    address: SocketAddr,
    dir: PathBuf,
}

impl PoolDeployment {
    /// Bring the node up, then the pool binary named by `STRATAMOTO_POOL`.
    pub fn start() -> Result<Self, Error> {
        let binary = std::env::var_os(POOL_BINARY_ENV).ok_or_else(|| {
            Error::Startup(format!(
                "{POOL_BINARY_ENV} does not name the pool binary to run"
            ))
        })?;
        Self::start_with(Path::new(&binary))
    }

    /// Bring the node up, then the pool binary at `binary`.
    pub fn start_with(binary: &Path) -> Result<Self, Error> {
        let node = Node::start()?;
        let pool = Pool::start(binary, &node)?;

        Ok(Self {
            pool,
            roles: vec![RoleConfig::new(Protocol::MiningProtocol)],
            node,
        })
    }

    /// Stop the pool and leave it stopped, as a crash would. For tests of what notices.
    pub fn kill_pool(&mut self) {
        self.pool.kill();
    }

    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.pool.address
    }

    /// Where the pool writes its log.
    #[must_use]
    pub fn log_path(&self) -> PathBuf {
        self.pool.dir.join("pool.log")
    }

    /// The node behind the pool, for a scenario that needs to move the chain tip.
    #[must_use]
    pub fn node(&self) -> &Node {
        &self.node
    }
}

impl Pool {
    fn start(binary: &Path, node: &Node) -> Result<Self, Error> {
        // The pool insists on binding its listener itself, so reserve a port by binding to it
        // and letting go, as sv2-apps' own tests do.
        let address = TcpListener::bind("127.0.0.1:0")?.local_addr()?;

        let dir = std::env::temp_dir().join(format!(
            "stratamoto-pool-{}-{}",
            std::process::id(),
            address.port()
        ));
        std::fs::create_dir_all(&dir)?;
        let config_path = dir.join("pool-config.toml");
        std::fs::write(&config_path, config(address, node.data_dir()))?;

        let log = File::create(dir.join("pool.log"))?;
        let child = Command::new(binary)
            .arg("--config")
            .arg(&config_path)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .spawn()
            .map_err(|e| Error::Startup(format!("could not start {}: {e}", binary.display())))?;

        let mut pool = Self {
            child,
            address,
            dir,
        };

        // The node's IPC socket is already there when it returns, so the only thing left to
        // wait on is the pool taking its first template and opening its door.
        if let Err(e) = pool.wait_until_accepting() {
            pool.kill();
            return Err(e);
        }
        Ok(pool)
    }

    /// The pool binds before its first template arrives but only accepts afterwards, so a TCP
    /// connection going through says nothing. A completed handshake does.
    fn wait_until_accepting(&mut self) -> Result<(), Error> {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Err(Error::Startup(format!(
                    "the pool exited with {status} before accepting a connection; its log is \
                     {}",
                    self.dir.join("pool.log").display()
                )));
            }
            match dial(self.address, CONNECT_RETRY_BUDGET) {
                Ok(_) => return Ok(()),
                Err(e) if Instant::now() >= deadline => {
                    return Err(Error::Startup(format!(
                        "the pool never accepted a connection: {e}"
                    )));
                }
                Err(_) => thread::sleep(Duration::from_millis(100)),
            }
        }
    }

    /// Whether the process is still running.
    fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.kill();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The pool's configuration file, in the shape its binary reads.
fn config(listen: SocketAddr, node_data_dir: &Path) -> String {
    format!(
        "listen_address = \"{listen}\"\n\
         authority_public_key = \"{AUTHORITY_PUBLIC_KEY_ENCODED}\"\n\
         authority_secret_key = \"{AUTHORITY_SECRET_KEY_ENCODED}\"\n\
         cert_validity_sec = {CERTIFICATE_VALIDITY_SECS}\n\
         coinbase_reward_script = \"{COINBASE_REWARD_DESCRIPTOR}\"\n\
         pool_signature = \"stratamoto\"\n\
         shares_per_minute = {SHARES_PER_MINUTE:?}\n\
         share_batch_size = 1\n\
         server_id = 1\n\
         \n\
         [template_provider_type.BitcoinCoreIpc]\n\
         version = {IPC_VERSION}\n\
         network = \"regtest\"\n\
         data_dir = \"{}\"\n\
         fee_threshold = 0\n\
         min_interval = {TEMPLATE_INTERVAL_SECS}\n",
        node_data_dir.display()
    )
}

impl Deployment for PoolDeployment {
    type Transport<'a> = NoiseTransport;

    fn connect(&self, _connection: usize, role: usize) -> Option<NoiseTransport> {
        if role >= self.roles.len() {
            return None;
        }
        // Under connection churn a fresh accept can be momentarily starved and refuse the
        // socket. That is a transient of talking to a real role, so it is retried briefly. The
        // long budget belongs to startup alone: a pool that is serving accepts at once, and
        // waiting 30 seconds on one that has stopped would cost that much on every connection
        // of every later run.
        dial(self.pool.address, CONNECT_RETRY_BUDGET)
            .map_err(|e| tracing::debug!("could not connect to the pool: {e}"))
            .ok()
    }

    fn role_config(&self, role: usize) -> Option<&RoleConfig> {
        self.roles.get(role)
    }

    fn num_roles(&self) -> usize {
        self.roles.len()
    }

    fn advance_time(&self, duration: Duration) {
        thread::sleep(duration);
    }

    fn is_alive(&self) -> bool {
        // The process's own status is not available through a shared reference, so this asks
        // the pool what a downstream would: whether a fresh handshake completes. A short
        // budget: a live pool answers at once, and a pool that exited never will, so there is
        // nothing to wait out.
        dial(self.pool.address, LIVENESS_TIMEOUT).is_ok()
    }
}

impl PoolDeployment {
    /// Whether the pool process is still running, regardless of whether it is serving.
    pub fn is_running(&mut self) -> bool {
        self.pool.is_running()
    }
}

/// Open one Noise connection to the pool, retrying transient starvation within `budget`.
fn dial(address: SocketAddr, budget: Duration) -> Result<NoiseTransport, stratamoto::error::Error> {
    let deadline = Instant::now() + budget;
    loop {
        let attempt = TcpStream::connect_timeout(&address, IO_TIMEOUT)
            .map_err(stratamoto::error::Error::Io)
            .and_then(|socket| NoiseTransport::connect(socket, IO_TIMEOUT));
        match attempt {
            Ok(transport) => return Ok(transport),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e),
        }
    }
}
