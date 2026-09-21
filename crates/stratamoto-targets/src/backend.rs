//! The restart backend: correct isolation first, fast isolation later.
//!
//! Every input runs on a pool that has served nothing, because the pool is replaced between
//! inputs. A canary checks that this holds: a fixed setup is run after each reset and what
//! the pool answers it has to be what it answered at the root, so a pool that did not come
//! back, or came back answering differently, is reported before an input is blamed for it.
//! What the canary cannot see is plain reuse: a fresh connection's setup is answered alike
//! whether or not the pool served before. The replacement is what guarantees isolation; the
//! canary guards the replacement.

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

use stratamoto::{
    backend::{ExecutionBackend, SnapshotId},
    digest::Verdict,
    runner,
    transport::Deployment,
};
use stratamoto_ir::{
    IndexedVariable, Operation, ProgramBuilder, ProgramContext, Protocol,
    compiler::{CompiledProgram, Compiler},
};

use crate::{Error, pool::PoolDeployment};

/// What is replaced between inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reset {
    /// The pool alone; the node and sv2-tp keep running.
    Pool,
    /// The pool, sv2-tp and the node.
    All,
    /// Nothing: every input sees what the ones before it left behind.
    None,
}

impl Reset {
    /// `pool`, `all` or `none`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "pool" => Some(Reset::Pool),
            "all" => Some(Reset::All),
            "none" => Some(Reset::None),
            _ => None,
        }
    }
}

/// sv2-apps' pool, replaced between inputs.
pub struct RestartBackend {
    deployment: Option<PoolDeployment>,
    reset: Reset,
    /// Whether the current pool has served an input and is due for replacement.
    served: bool,
    /// The canary and what it answered at the root, once the root was taken.
    canary: Option<CompiledProgram>,
    root: Option<u64>,
    /// Run the canary before every `canary_every`th input; zero turns it off.
    canary_every: usize,
    inputs: usize,
    snapshot: Option<SnapshotId>,
}

impl RestartBackend {
    #[must_use]
    pub fn new(reset: Reset) -> Self {
        Self {
            deployment: None,
            reset,
            served: false,
            canary: Some(canary()),
            root: None,
            canary_every: 1,
            inputs: 0,
            snapshot: None,
        }
    }

    /// Run the canary before every `every`th input; zero never runs it.
    #[must_use]
    pub fn with_canary_every(mut self, every: usize) -> Self {
        self.canary_every = every;
        self
    }

    #[must_use]
    pub fn reset_policy(&self) -> Reset {
        self.reset
    }

    /// Change what is replaced between inputs. Before the root is taken, since the root's
    /// identity names the policy.
    pub fn set_reset(&mut self, reset: Reset) {
        self.reset = reset;
    }

    /// The deployment, whatever state it is in. For callers that ask about it, not run on it.
    #[must_use]
    pub fn deployment(&self) -> Option<&PoolDeployment> {
        self.deployment.as_ref()
    }

    /// Whether the backend can still hand out inputs: with a reset policy it always can, since
    /// a pool that stopped serving is replaced; without one it is tied to the pool's fate.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        self.reset != Reset::None || self.deployment.as_ref().is_some_and(Deployment::is_alive)
    }

    /// The deployment, to do something to it that an input would not. For tests of what the
    /// canary notices.
    pub fn deployment_mut(&mut self) -> Result<&mut PoolDeployment, Error> {
        self.deployment
            .as_mut()
            .ok_or_else(|| Error::Startup("the backend was not prepared".to_string()))
    }

    /// Replace what the policy says to replace.
    fn reset(&mut self) -> Result<(), Error> {
        match self.reset {
            Reset::Pool => self.deployment_mut()?.restart_pool()?,
            Reset::All => self.deployment_mut()?.restart_all()?,
            Reset::None => {}
        }
        self.served = false;
        Ok(())
    }

    /// What the pool answers the canary.
    fn observe(&mut self) -> Result<u64, Error> {
        let (Some(canary), Some(deployment)) = (&self.canary, &self.deployment) else {
            return Ok(0);
        };
        let execution = runner::run(deployment, canary);
        let mut hasher = DefaultHasher::new();
        let mut sessions: Vec<_> = execution.sessions.iter().collect();
        sessions.sort_by_key(|(id, _)| **id);
        for (id, session) in sessions {
            id.hash(&mut hasher);
            format!("{:?}", session.response).hash(&mut hasher);
        }
        Ok(hasher.finish())
    }
}

impl ExecutionBackend for RestartBackend {
    type Deployment = PoolDeployment;
    type Error = Error;

    fn prepare_scenario(&mut self) -> Result<(), Error> {
        self.deployment = Some(PoolDeployment::start()?);
        self.served = false;
        Ok(())
    }

    fn take_root_snapshot(&mut self) -> Result<SnapshotId, Error> {
        // The canary is an input like any other: what it observes at the root is the root,
        // and the pool it observed is replaced before the first real input.
        let root = self.observe()?;
        self.root = Some(root);
        self.served = self.canary.is_some();
        let snapshot = SnapshotId {
            backend: "restart".to_string(),
            identity: format!("{:?}/{root:016x}", self.reset),
        };
        self.snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    fn next_input(&mut self) -> Result<&PoolDeployment, Error> {
        if self.served {
            self.reset()?;
        }
        self.inputs += 1;
        if self.canary_every != 0
            && self.inputs.is_multiple_of(self.canary_every)
            && let Some(root) = self.root
        {
            let now = self.observe()?;
            if now != root {
                return Err(Error::Startup(format!(
                    "reset integrity: the pool answered the canary {now:016x} where the root \
                     answered {root:016x}, so the input would not start from the root"
                )));
            }
            // The canary has now served on this pool, so the input gets a fresh one.
            if self.reset != Reset::None {
                self.reset()?;
            }
        }
        self.deployment
            .as_ref()
            .ok_or_else(|| Error::Startup("the backend was not prepared".to_string()))
    }

    fn report_and_reset(&mut self, _verdict: &Verdict) -> Result<(), Error> {
        // The reset itself is deferred to the next input, so that the last input of a
        // campaign does not pay for a pool nothing will use.
        self.served = true;
        Ok(())
    }

    fn snapshot(&self) -> Option<&SnapshotId> {
        self.snapshot.as_ref()
    }
}

/// A mining setup: what a pool at the root answers it with is what every input starts from.
fn canary() -> CompiledProgram {
    fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
        variables.remove(0)
    }
    let mut b = ProgramBuilder::new(ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    });
    let role = one(b.append_op(Operation::LoadRole(0), &[]).expect("valid"));
    let connection = one(b.append_op(Operation::Connect, &[&role]).expect("valid"));
    let setup = one(b
        .append_op(Operation::BeginBuildSetupConnection, &[])
        .expect("valid"));
    let setup = one(b
        .append_op(
            Operation::EndBuildSetupConnection {
                protocol: Protocol::Mining,
            },
            &[&setup],
        )
        .expect("valid"));
    b.append_op(
        Operation::SendSetupConnection {
            protocol: Protocol::Mining,
        },
        &[&connection, &setup],
    )
    .expect("valid");
    Compiler::new()
        .compile(&b.finalize().expect("valid"))
        .expect("compiles")
}
