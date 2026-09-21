use stratamoto_ir::{
    Protocol,
    compiler::{Action, CompiledProgram, SetupConnectionSpec},
};

use crate::{
    roles::{RoleConfig, protocol_of},
    runner::{ActionId, ActionOutcome, Execution, Response, SetupResponse},
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
///
/// An oracle walks the compiled actions and looks each one up in the outcomes, never the
/// other way round: what the program asked for is the expectation, and an action the run has
/// no answer for is a failure of the harness, not a pass.
pub trait Oracle {
    fn evaluate<D: Deployment>(
        &self,
        deployment: &D,
        program: &CompiledProgram,
        execution: &Execution,
    ) -> OracleResult;

    fn name(&self) -> &'static str;
}

/// The run recorded what it was supposed to, and nothing in it failed on the harness's side.
///
/// A failure here is about the harness, so a scenario reports it as an infrastructure verdict
/// and never as a finding about the deployment. It runs before every other oracle, since the
/// others assume what it checks.
pub struct HarnessIntegrityOracle;

impl Oracle for HarnessIntegrityOracle {
    fn evaluate<D: Deployment>(
        &self,
        _deployment: &D,
        program: &CompiledProgram,
        execution: &Execution,
    ) -> OracleResult {
        if !execution.is_total(program) {
            return OracleResult::Fail(format!(
                "the program has {} actions and the execution {} outcomes",
                program.actions.len(),
                execution.outcomes.len()
            ));
        }
        for (id, outcome) in execution.outcomes.iter().enumerate() {
            if let ActionOutcome::HarnessError(e) = outcome {
                return OracleResult::Fail(format!(
                    "the harness could not carry out action {id} ({}): {e}",
                    describe(program, id)
                ));
            }
        }
        OracleResult::Pass
    }

    fn name(&self) -> &'static str {
        "HarnessIntegrityOracle"
    }
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
        for (id, action) in program.actions.iter().enumerate() {
            let Action::AwaitSetupResponse {
                connection,
                session,
                first_on_connection,
                ..
            } = action
            else {
                continue;
            };
            let Some(outcome) = execution.outcome(id) else {
                return OracleResult::Fail(format!(
                    "action {id} ({}) has no outcome",
                    describe(program, id)
                ));
            };
            let Some(spec) = program.metadata.session_setups.get(session) else {
                return OracleResult::Fail(format!(
                    "session {session} has no recorded SetupConnection"
                ));
            };
            let Some(role) = execution.connection_roles.get(connection) else {
                // The connection was never opened, so nothing was asked of any role. Whether
                // it should have opened is the crash oracle's question.
                continue;
            };
            let Some(config) = deployment.role_config(*role) else {
                return OracleResult::Fail(format!("session {session} is on unknown role {role}"));
            };

            // An answer is owed only to a setup the client sent as the protocol requires: the
            // first message on its connection, with nothing of its own choosing before it.
            let owed = *first_on_connection && !program.metadata.violations.contains_key(&id);
            if let Err(violation) = check(config, spec, owed, outcome) {
                return OracleResult::Fail(format!(
                    "role {role} answered {outcome:?} to {spec:?}: {violation}"
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

/// The instruction an action came from, for a message about it.
fn describe(program: &CompiledProgram, action: ActionId) -> String {
    match program.metadata.action_instructions.get(action) {
        Some(instruction) => format!("instruction {instruction}"),
        None => "no instruction".to_string(),
    }
}

/// Why `outcome` is not an answer to `spec` that a role configured as `config` may give.
///
/// `owed` says whether the server owed an answer at all: only to a `SetupConnection` that was
/// the first message on its connection, since one sent later is already the client's
/// violation. The server may ignore it or close the connection, and nothing it does in reply
/// is constrained.
pub fn check(
    config: &RoleConfig,
    spec: &SetupConnectionSpec,
    owed: bool,
    outcome: &ActionOutcome,
) -> Result<(), String> {
    if !owed {
        return Ok(());
    }

    let response = match outcome {
        ActionOutcome::Completed(Response::Setup(response)) => response,
        ActionOutcome::TimedOut => {
            return Err(
                "the server MUST respond with SetupConnection.Success or SetupConnection.Error"
                    .to_string(),
            );
        }
        ActionOutcome::TransportClosed => {
            return Err("the server closed the connection instead of answering".to_string());
        }
        ActionOutcome::TransportError(e) => {
            return Err(format!(
                "the transport failed before an answer arrived: {e}"
            ));
        }
        // Nothing was sent, so nothing is owed; and what the harness could not do, the
        // integrity oracle reports.
        ActionOutcome::Skipped(_) | ActionOutcome::HarnessError(_) => return Ok(()),
        ActionOutcome::Completed(other) => {
            return Err(format!("the outcome is of the wrong kind: {other:?}"));
        }
    };

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
