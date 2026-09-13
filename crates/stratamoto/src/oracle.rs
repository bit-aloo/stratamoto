use stratamoto_ir::compiler::{CompiledProgram, SetupConnectionSpec};

use crate::{
    transport::Deployment,
    roles::{RoleConfig, upstream::ERROR_CODE_PROTOCOL_VERSION_MISMATCH, protocol_of},
    runner::{Execution, SetupResponse},
};

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

/// Every `SetupConnection` is answered, and answered the way section 3.6 requires.
///
/// The expectation is derived from the specification rather than from the role's own code,
/// so that the check still means something once the role is a real implementation.
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

            let expected = expected_response(config, spec);
            if session.response != expected {
                return OracleResult::Fail(format!(
                    "role {role} answered {:?} to {spec:?}, expected {expected:?}",
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

/// The answer the common protocol requires, per specification section 3.6.
///
/// A server that cannot set up the connection answers `SetupConnection.Error` before closing,
/// and must report the full set of flags it does not support. `flags` is 0 when the refusal
/// has another cause.
#[must_use]
pub fn expected_response(config: &RoleConfig, spec: &SetupConnectionSpec) -> SetupResponse {
    use stratum_core::common_messages_sv2::{
        ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_FEATURE_FLAGS,
        ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_PROTOCOL,
    };

    if protocol_of(spec.protocol) != config.protocol {
        return SetupResponse::Error {
            flags: 0,
            error_code: ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_PROTOCOL.to_string(),
        };
    }

    // Version negotiation picks the highest version both ends support.
    let used_version = spec.max_version.min(config.max_version);
    if used_version < spec.min_version || used_version < config.min_version {
        return SetupResponse::Error {
            flags: 0,
            error_code: ERROR_CODE_PROTOCOL_VERSION_MISMATCH.to_string(),
        };
    }

    let unsupported = spec.flags & !config.supported_flags;
    if unsupported != 0 {
        return SetupResponse::Error {
            flags: unsupported,
            error_code: ERROR_CODE_SETUP_CONNECTION_UNSUPPORTED_FEATURE_FLAGS.to_string(),
        };
    }

    SetupResponse::Success {
        used_version,
        flags: config.supported_flags,
    }
}
