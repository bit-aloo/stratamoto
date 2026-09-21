use stratamoto::{
    ir::{Protocol, compiler::SetupConnectionSpec},
    oracle::{MINING_REQUIRES_VERSION_ROLLING, MINING_SUCCESS_REQUIRES_FIXED_VERSION, check},
    roles::RoleConfig,
    runner::{ActionOutcome, Response, SetupResponse},
    stratum_core::common_messages_sv2::Protocol as WireProtocol,
};

fn pool() -> RoleConfig {
    RoleConfig::new(WireProtocol::MiningProtocol)
}

fn setup(
    protocol: Protocol,
    min_version: u16,
    max_version: u16,
    flags: u32,
) -> SetupConnectionSpec {
    SetupConnectionSpec {
        protocol,
        min_version,
        max_version,
        flags,
        ..SetupConnectionSpec::default()
    }
}

fn answered(response: SetupResponse) -> ActionOutcome {
    ActionOutcome::Completed(Response::Setup(response))
}

#[test]
fn an_unanswered_setup_connection_fails() {
    assert!(
        check(
            &pool(),
            &setup(Protocol::Mining, 2, 2, 0),
            true,
            &ActionOutcome::TimedOut
        )
        .is_err()
    );
}

#[test]
fn a_connection_closed_before_the_answer_fails() {
    assert!(
        check(
            &pool(),
            &setup(Protocol::Mining, 2, 2, 0),
            true,
            &ActionOutcome::TransportClosed
        )
        .is_err()
    );
}

/// Nothing was sent, so nothing is owed: a setup on a connection that never opened is not a
/// conformance question.
#[test]
fn a_setup_that_was_never_sent_passes() {
    use stratamoto::runner::Prerequisite;
    let skipped = ActionOutcome::Skipped(Prerequisite::ConnectionOpen(0));
    assert!(check(&pool(), &setup(Protocol::Mining, 2, 2, 0), true, &skipped).is_ok());
}

#[test]
fn an_answer_that_is_neither_success_nor_error_fails() {
    let response = answered(SetupResponse::Unexpected { message_type: 0x10 });
    assert!(check(&pool(), &setup(Protocol::Mining, 2, 2, 0), true, &response).is_err());
}

/// Section 3.5 leaves error codes to each implementation.
#[test]
fn any_error_code_is_accepted() {
    let response = answered(SetupResponse::Error {
        flags: 0,
        error_code: "a-code-no-other-implementation-uses".to_string(),
    });
    assert!(check(&pool(), &setup(Protocol::Mining, 2, 2, 0), true, &response).is_ok());
}

#[test]
fn a_version_the_client_did_not_propose_fails() {
    let response = answered(SetupResponse::Success {
        used_version: 2,
        flags: 0,
    });
    assert!(check(&pool(), &setup(Protocol::Mining, 3, 2, 0), true, &response).is_err());
}

#[test]
fn fixed_version_against_a_client_requiring_version_rolling_fails() {
    let response = answered(SetupResponse::Success {
        used_version: 2,
        flags: MINING_SUCCESS_REQUIRES_FIXED_VERSION,
    });
    let spec = setup(Protocol::Mining, 2, 2, MINING_REQUIRES_VERSION_ROLLING);
    assert!(check(&pool(), &spec, true, &response).is_err());
}

#[test]
fn accepting_a_subprotocol_the_role_does_not_serve_fails() {
    let response = answered(SetupResponse::Success {
        used_version: 2,
        flags: 0,
    });
    assert!(
        check(
            &pool(),
            &setup(Protocol::TemplateDistribution, 2, 2, 0),
            true,
            &response
        )
        .is_err()
    );
}

/// What sv2-apps' pool does at the revision this harness tracks: it accepts flags it does not
/// act on, and answers a work selection request with REQUIRES_EXTENDED_CHANNELS. Neither is a
/// violation, and the oracle must not report either as one.
#[test]
fn the_real_pool_contract_passes() {
    let work_selection = 1 << 1;
    let undefined = 1 << 17;
    let response = answered(SetupResponse::Success {
        used_version: 2,
        flags: 1 << 1,
    });
    let spec = setup(Protocol::Mining, 2, 2, work_selection | undefined);
    assert_eq!(check(&pool(), &spec, true, &response), Ok(()));
}

/// Only the first message on a connection is owed an answer, so a later SetupConnection going
/// unanswered, as it does when the server has closed the connection, is not a violation.
#[test]
fn silence_after_the_first_message_passes() {
    let spec = setup(Protocol::Mining, 2, 2, 0);
    assert!(check(&pool(), &spec, false, &ActionOutcome::TimedOut).is_ok());
}
