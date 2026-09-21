use std::path::{Path, PathBuf};

use integration_tests_sv2::{
    template_provider::{BITCOIN_CORE_LATEST, BitcoinCore, DifficultyLevel},
    utils::get_available_address,
};

use crate::Error;

/// The authority key pair sv2-apps' own tests give their pool, base58check encoded.
pub(crate) const AUTHORITY_PUBLIC_KEY_ENCODED: &str =
    "9auqWEzQDVyd2oe1JVGFLMLHZtCo2FFqZwtKA5gd9xbuEu7PH72";
pub(crate) const AUTHORITY_SECRET_KEY_ENCODED: &str =
    "mkDLTBBRxdBv998612qipDYoTK3YUrqLe8uWw7gu3iXbSrn2n";

/// The major version of the Core release `BITCOIN_CORE_LATEST` stands for, which is how the
/// pool's configuration names the IPC schema to speak.
pub(crate) const IPC_VERSION: u8 = 31;

/// Blocks mined before anything else, so the node leaves initial block download and has
/// coinbases behind it. sv2-apps mines sixteen rather than one to work around a Core issue,
/// and `BitcoinCore::start` does not do it: their `start_bitcoin_core` does.
const STARTUP_BLOCKS: usize = 16;

/// A real Bitcoin Core node with IPC enabled, which the pool takes its templates from directly:
/// the pool carries `bitcoin-core-sv2`, so nothing stands between the two.
///
/// Templates come from a node rather than from us. A synthesized template only has to satisfy
/// the decoder, so a role could accept one no real node would ever produce, and nothing that
/// depends on a template's contents, a job built from it or a share against that job, means
/// anything until the template is real.
///
/// The node runs in regtest, where the target is low enough that a submitted share is also a
/// block, which is what makes share submission observable at all.
pub struct Node {
    inner: BitcoinCore,
}

impl Node {
    pub fn start() -> Result<Self, Error> {
        use_existing_binaries()?;

        // The port only names the node's wallet and data directory; nothing listens on it.
        let inner = BitcoinCore::start(
            get_available_address().port(),
            DifficultyLevel::Low,
            BITCOIN_CORE_LATEST,
        );
        inner.generate_blocks(STARTUP_BLOCKS);

        Ok(Self { inner })
    }

    /// The node's data directory, under which its IPC socket is: what the pool is pointed at.
    #[must_use]
    pub fn data_dir(&self) -> &Path {
        self.inner.data_dir()
    }

    /// Mine `n` blocks, which is how a test moves the chain tip and draws a new template.
    pub fn generate_blocks(&self, n: usize) {
        self.inner.generate_blocks(n);
    }

    /// Put a transaction in the node's mempool, so the next template is not just a coinbase.
    pub fn create_mempool_transaction(&self) -> Result<(), Error> {
        self.inner
            .create_mempool_transaction()
            .map(|_| ())
            .map_err(|e| Error::Startup(format!("could not create a mempool transaction: {e}")))
    }

    pub fn fund_wallet(&self) -> Result<(), Error> {
        self.inner
            .fund_wallet()
            .map_err(|e| Error::Startup(format!("could not fund the wallet: {e}")))
    }
}

/// sv2-apps resolves its Bitcoin Core binaries from `template-provider` next to the working
/// directory, and downloads them there when they are missing. Point that at a copy that
/// already has them, so a run does not fetch Bitcoin Core again.
fn use_existing_binaries() -> Result<(), Error> {
    let local = std::env::current_dir()?.join("template-provider");
    if local.exists() {
        return Ok(());
    }
    // Without a cache the launcher downloads, which works and is simply slow the first time.
    let Some(cache) = cached_binaries() else {
        return Ok(());
    };
    // Two deployments starting at once both find it missing; whichever links second is fine.
    match std::os::unix::fs::symlink(&cache, &local) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn cached_binaries() -> Option<PathBuf> {
    if let Ok(configured) = std::env::var("STRATAMOTO_TEMPLATE_PROVIDER_CACHE") {
        let configured = PathBuf::from(configured);
        return configured.exists().then_some(configured);
    }

    // An sv2-apps checkout beside this one is where these binaries usually already are.
    let mut directory = std::env::current_dir().ok()?;
    loop {
        let candidate = directory.join("sv2-apps/integration-tests/template-provider");
        if candidate.exists() {
            return candidate.canonicalize().ok();
        }
        if !directory.pop() {
            return None;
        }
    }
}
