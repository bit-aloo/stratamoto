use stratamoto_ir::{
    Protocol,
    compiler::{CompiledProgram, SetupConnectionSpec},
};

use crate::{
    roles::{RoleConfig, protocol_of},
    runner::{Execution, SetupResponse},
    transport::Deployment,
};

/// `REQUIRES_VERSION_ROLLING`, bit 2 of a mining `SetupConnection.flags` (section 5.3.1).
pub const MINING_REQUIRES_VERSION_ROLLING: u32 = 1 << 2;
/// `REQUIRES_FIXED_VERSION`, bit 0 of a mining `SetupConnection.Success.flags` (section 5.3.1).
pub const MINING_SUCCESS_REQUIRES_FIXED_VERSION: u32 = 1 << 0;

pub enum OracleResult {
    Pass,
    Fail(String),
}

/// Checks an execution against a property the protocol requires of any implementation.
pub trait Oracle {
    fn evaluate<D: Deployment>(
        &self,
        deployment: &D,
        program: &CompiledProgram,
        execution: &Execution,
    ) -> OracleResult;

    fn name(&self) -> &'static str;
}

/// Every `SetupConnection` is answered, and answered in a way the specification allows.
///
/// Error codes are deliberately not compared: section 3.5 lets each implementation choose its
/// own, so asserting on one would test an implementation's choices rather than its
/// conformance. Likewise a server may accept flags it does not act on; only an explicit
/// requirement of the specification is a failure.
pub struct SetupConnectionOracle;

impl Oracle for SetupConnectionOracle {
    fn evaluate<D: Deployment>(
        &self,
        deployment: &D,
        program: &CompiledProgram,
        execution: &Execution,
    ) -> OracleResult {
        for (id, session) in &execution.sessions {
            let Some(spec) = program.metadata.session_setups.get(id) else {
                return OracleResult::Fail(format!("session {id} has no recorded SetupConnection"));
            };
            let Some(role) = execution.connection_roles.get(&session.connection) else {
                return OracleResult::Fail(format!(
                    "session {id} is on connection {} which was never opened",
                    session.connection
                ));
            };
            let Some(config) = deployment.role_config(*role) else {
                return OracleResult::Fail(format!("session {id} is on unknown role {role}"));
            };

            if let Err(violation) =
                check(config, spec, session.first_on_connection, &session.response)
            {
                return OracleResult::Fail(format!(
                    "role {role} answered {:?} to {spec:?}: {violation}",
                    session.response
                ));
            }
        }

        OracleResult::Pass
    }

    fn name(&self) -> &'static str {
        "SetupConnectionOracle"
    }
}

/// Checks that the deployment is still serving after a program has run.
///
/// A conformant role stays up no matter what a client sends it: a malformed or out of order
/// message is the client's problem, never grounds to stop serving every other client. A role
/// that is no longer reachable after a run has failed this, whether it crashed or shut itself
/// down.
pub struct CrashOracle;

impl Oracle for CrashOracle {
    fn evaluate<D: Deployment>(
        &self,
        deployment: &D,
        _program: &CompiledProgram,
        _execution: &Execution,
    ) -> OracleResult {
        if deployment.is_alive() {
            OracleResult::Pass
        } else {
            OracleResult::Fail("the deployment stopped serving".to_string())
        }
    }

    fn name(&self) -> &'static str {
        "CrashOracle"
    }
}

/// Why `response` is not an answer to `spec` that a role configured as `config` may give.
pub fn check(
    config: &RoleConfig,
    spec: &SetupConnectionSpec,
    first_on_connection: bool,
    response: &SetupResponse,
) -> Result<(), String> {
    // SetupConnection MUST be the first message on a new connection, and it is that message the
    // server MUST answer. One sent later is already the client's violation: the server may
    // ignore it or close the connection, and nothing it does in reply is constrained.
    if !first_on_connection {
        return Ok(());
    }

    let (used_version, flags) = match response {
        SetupResponse::Silence => {
            return Err(
                "the server MUST respond with SetupConnection.Success or SetupConnection.Error"
                    .to_string(),
            );
        }
        SetupResponse::Unexpected { message_type } => {
            return Err(format!(
                "the answer has message type 0x{message_type:02x}, which is neither \
                 SetupConnection.Success nor SetupConnection.Error"
            ));
        }
        SetupResponse::Error { .. } => return Ok(()),
        SetupResponse::Success {
            used_version,
            flags,
        } => (*used_version, *flags),
    };

    if !(spec.min_version..=spec.max_version).contains(&used_version) {
        return Err(format!(
            "used_version {used_version} is not a version the client proposed ({}..={})",
            spec.min_version, spec.max_version
        ));
    }

    if spec.protocol == Protocol::Mining
        && flags & MINING_SUCCESS_REQUIRES_FIXED_VERSION != 0
        && spec.flags & MINING_REQUIRES_VERSION_ROLLING != 0
    {
        return Err(
            "REQUIRES_FIXED_VERSION MUST NOT be set when the client set REQUIRES_VERSION_ROLLING"
                .to_string(),
        );
    }

    // What follows is not the specification but what the deployment says of the role: a role
    // that accepts a subprotocol it does not serve, or settles on a version it does not
    // support, has agreed to something it cannot do.
    if protocol_of(spec.protocol) != config.protocol {
        return Err(format!(
            "accepted a {} connection on a role that serves {:?}",
            spec.protocol, config.protocol
        ));
    }

    if !(config.min_version..=config.max_version).contains(&used_version) {
        return Err(format!(
            "used_version {used_version} is not a version the role supports ({}..={})",
            config.min_version, config.max_version
        ));
    }

    Ok(())
}
