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
    noise::NoiseTransport,
    roles::RoleConfig,
    stratum_core::common_messages_sv2::Protocol,
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
    _template_provider: TemplateProvider,
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

        template_provider.wait_until_served(STARTUP_TIMEOUT)?;
        wait_until_accepting(address)?;

        Ok(Self {
            runtime,
            pool,
            address,
            roles: vec![RoleConfig::new(Protocol::MiningProtocol)],
            _template_provider: template_provider,
        })
    }

    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }
}

impl Deployment for PoolDeployment {
    type Transport<'a> = NoiseTransport;

    fn connect(&self, _connection: usize, role: usize) -> Option<NoiseTransport> {
        if role >= self.roles.len() {
            return None;
        }
        let socket = TcpStream::connect_timeout(&self.address, IO_TIMEOUT).ok()?;
        NoiseTransport::connect(socket, IO_TIMEOUT)
            .map_err(|e| log::debug!("handshake with the pool failed: {e}"))
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
}

impl Drop for PoolDeployment {
    fn drop(&mut self) {
        self.runtime.block_on(self.pool.shutdown());
    }
}

/// The pool binds before its first template arrives but only accepts afterwards, so a TCP
/// connection going through says nothing. A completed handshake does.
fn wait_until_accepting(address: SocketAddr) -> Result<(), Error> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let attempt = TcpStream::connect_timeout(&address, IO_TIMEOUT)
            .map_err(stratamoto::error::Error::Io)
            .and_then(|socket| NoiseTransport::connect(socket, Duration::from_secs(2)));
        match attempt {
            Ok(_) => return Ok(()),
            Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(250)),
            Err(e) => {
                return Err(Error::Startup(format!(
                    "the pool never accepted a connection: {e}"
                )));
            }
        }
    }
}
