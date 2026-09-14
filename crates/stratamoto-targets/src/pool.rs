use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

use pool_sv2::{
    PoolSv2,
    config::{AuthorityConfig, ConnectionConfig, PoolConfig},
};
use stratamoto::{
    noise::NoiseTransport, roles::RoleConfig, stratum_core::common_messages_sv2::Protocol,
    transport::Deployment,
};
use stratum_apps::{
    config_helpers::CoinbaseRewardScript,
    key_utils::{Secp256k1PublicKey, Secp256k1SecretKey},
    tp_type::TemplateProviderType,
};

use crate::{
    Error,
    template_provider::{
        AUTHORITY_PUBLIC_KEY_ENCODED, AUTHORITY_SECRET_KEY_ENCODED, TemplateProvider,
    },
};

const COINBASE_REWARD_DESCRIPTOR: &str = "addr(tb1qa0sm0hxzj0x25rh8gw5xlzwlsfvvyz8u96w3p8)";
const SHARES_PER_MINUTE: f32 = 120.0;
const CERTIFICATE_VALIDITY_SECS: u64 = 3600;

/// The pool looks for its first template once a second, so readiness takes at least that long.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// A pool that is up answers a handshake immediately, so a liveness probe waits only briefly.
const LIVENESS_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a connection to an already running pool is retried through transient starvation.
const CONNECT_RETRY_BUDGET: Duration = Duration::from_secs(3);

/// sv2-apps' pool, running in process against a Template Provider the harness plays.
///
/// The pool keeps its own tokio runtime and real sockets, so unlike a simulated deployment a
/// run is only as reproducible as the pool itself. It is started once and reused: every
/// connection a program opens is a fresh TCP connection with its own handshake.
pub struct PoolDeployment {
    runtime: tokio::runtime::Runtime,
    pool: PoolSv2,
    address: SocketAddr,
    roles: Vec<RoleConfig>,
    template_provider: TemplateProvider,
}

impl PoolDeployment {
    pub fn start() -> Result<Self, Error> {
        let template_provider = TemplateProvider::start()?;

        // The pool insists on binding its listener itself, so reserve a port by binding to it
        // and letting go, as sv2-apps' own tests do.
        let address = TcpListener::bind("127.0.0.1:0")?.local_addr()?;

        let config = PoolConfig::new(
            ConnectionConfig::new(address, CERTIFICATE_VALIDITY_SECS, "stratamoto".to_string()),
            TemplateProviderType::Sv2Tp {
                address: template_provider.address().to_string(),
                public_key: None,
            },
            AuthorityConfig::new(
                Secp256k1PublicKey::try_from(AUTHORITY_PUBLIC_KEY_ENCODED.to_string())
                    .map_err(|e| Error::Startup(format!("authority public key: {e:?}")))?,
                Secp256k1SecretKey::try_from(AUTHORITY_SECRET_KEY_ENCODED.to_string())
                    .map_err(|e| Error::Startup(format!("authority secret key: {e:?}")))?,
            ),
            CoinbaseRewardScript::from_descriptor(COINBASE_REWARD_DESCRIPTOR)
                .map_err(|e| Error::Startup(format!("coinbase reward script: {e:?}")))?,
            SHARES_PER_MINUTE,
            1,
            1,
            vec![],
            vec![],
            None,
            None,
            None,
        );

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let pool = PoolSv2::new(config);
        let starting = pool.clone();
        runtime.spawn(async move {
            if let Err(e) = starting.start().await {
                log::error!("pool stopped: {e:?}");
            }
        });

        // The Template Provider is already serving when it returns, so the only thing left to
        // wait on is the pool taking its first template and opening its door.
        wait_until_accepting(address)?;

        Ok(Self {
            runtime,
            pool,
            address,
            roles: vec![RoleConfig::new(Protocol::MiningProtocol)],
            template_provider,
        })
    }

    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The node behind the pool, for a scenario that needs to move the chain tip.
    #[must_use]
    pub fn template_provider(&self) -> &TemplateProvider {
        &self.template_provider
    }
}

impl Deployment for PoolDeployment {
    type Transport<'a> = NoiseTransport;

    fn connect(&self, _connection: usize, role: usize) -> Option<NoiseTransport> {
        if role >= self.roles.len() {
            return None;
        }
        // The pool runs on a shared runtime, so under connection churn a fresh accept can be
        // momentarily starved and refuse the socket. That is a transient of talking to a real
        // role, so it is retried briefly. The long budget belongs to startup alone: a pool
        // that is serving accepts at once, and waiting 30 seconds on one that has stopped
        // would cost that much on every connection of every later run.
        dial(self.address, CONNECT_RETRY_BUDGET)
            .map_err(|e| log::debug!("could not connect to the pool: {e}"))
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
        // A short budget: a live pool answers a handshake at once, and a pool that shut itself
        // down never will, so there is nothing to wait out.
        dial(self.address, LIVENESS_TIMEOUT).is_ok()
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

impl Drop for PoolDeployment {
    fn drop(&mut self) {
        self.runtime.block_on(self.pool.shutdown());
    }
}

/// The pool binds before its first template arrives but only accepts afterwards, so a TCP
/// connection going through says nothing. A completed handshake does.
fn wait_until_accepting(address: SocketAddr) -> Result<(), Error> {
    dial(address, STARTUP_TIMEOUT)
        .map(|_| ())
        .map_err(|e| Error::Startup(format!("the pool never accepted a connection: {e}")))
}
