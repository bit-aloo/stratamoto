//! Every frame a connection receives goes through one dispatcher, so an answer is correlated
//! with its request whatever else arrived in between.

use stratamoto::{
    connection::{Frame, assemble},
    deployment::SimulatedDeployment,
    events::{ConnectionState, Event, SetupState},
    roles::RoleConfig,
    runner::{self, ActionOutcome, Response, SetupResponse},
    stratum_core::{
        common_messages_sv2::{Protocol as WireProtocol, Reconnect, SetupConnectionSuccess},
        parsers_sv2::{AnyMessage, CommonMessages},
    },
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::Compiler,
};

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

/// Two mining setups on two connections, then a probe.
fn two_setups() -> Program {
    let mut b = ProgramBuilder::new(ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    });
    let role = one(b.append_op(Operation::LoadRole(0), &[]).unwrap());
    for _ in 0..2 {
        let connection = one(b.append_op(Operation::Connect, &[&role]).unwrap());
        let setup = one(b
            .append_op(Operation::BeginBuildSetupConnection, &[])
            .unwrap());
        let setup = one(b
            .append_op(
                Operation::EndBuildSetupConnection {
                    protocol: Protocol::Mining,
                },
                &[&setup],
            )
            .unwrap());
        b.append_op(
            Operation::SendSetupConnection {
                protocol: Protocol::Mining,
            },
            &[&connection, &setup],
        )
        .unwrap();
    }
    b.append_op(Operation::Probe, &[]).unwrap();
    b.finalize().unwrap()
}

/// Each connection's setup is answered on that connection, and the trace of each holds its
/// own answer and nothing else.
#[test]
fn each_connection_keeps_its_own_trace() {
    let deployment =
        SimulatedDeployment::new(1, vec![RoleConfig::new(WireProtocol::MiningProtocol)]);
    let compiled = Compiler::new().compile(&two_setups()).unwrap();
    let execution = runner::run(&deployment, &compiled);

    for connection in 0..2 {
        let state = &execution.connections[&connection];
        assert_eq!(
            state.setup,
            SetupState::Established {
                used_version: 2,
                flags: 0
            }
        );
        assert_eq!(
            state.events,
            vec![Event::SetupSuccess {
                used_version: 2,
                flags: 0
            }]
        );
        assert!(state.header_issues.is_empty());
    }
    let setups: Vec<_> = execution
        .outcomes
        .iter()
        .filter(|o| matches!(o, ActionOutcome::Completed(Response::Setup(_))))
        .collect();
    assert_eq!(setups.len(), 2);
    assert!(execution.unsolicited.is_empty());
}

/// The first frame after a setup that is not its answer is what the setup got; an answer
/// that arrives when no setup is pending is kept as an event and settles nothing.
#[test]
fn a_setup_is_answered_by_the_first_frame_after_it() {
    let mut state = ConnectionState {
        setup: SetupState::Pending,
        ..Default::default()
    };
    state.observe(
        &AnyMessage::Common(CommonMessages::Reconnect(Reconnect {
            new_host: "elsewhere".try_into().unwrap(),
            new_port: 1,
        })),
        0x04,
    );
    assert_eq!(state.setup, SetupState::Unexpected { message_type: 0x04 });

    state.observe(
        &AnyMessage::Common(CommonMessages::SetupConnectionSuccess(
            SetupConnectionSuccess {
                used_version: 2,
                flags: 0,
            },
        )),
        0x01,
    );
    assert_eq!(
        state.setup,
        SetupState::Unexpected { message_type: 0x04 },
        "a late success does not rewrite what the setup got"
    );
    assert_eq!(
        state.events.len(),
        0,
        "observe classifies; dispatch records"
    );

    let mut fresh = ConnectionState::default();
    let event = fresh.observe(
        &AnyMessage::Common(CommonMessages::SetupConnectionSuccess(
            SetupConnectionSuccess {
                used_version: 2,
                flags: 0,
            },
        )),
        0x01,
    );
    assert_eq!(
        event,
        Event::SetupSuccess {
            used_version: 2,
            flags: 0
        }
    );
    assert_eq!(fresh.setup, SetupState::Unsent, "nothing was pending");
}

/// Unknown and malformed frames are kept as what they are rather than collapsed or dropped,
/// and a header that disagrees with its message is noted.
#[test]
fn unknown_and_malformed_frames_are_kept_in_the_trace() {
    let mut state = ConnectionState::default();

    // A message type nothing defines.
    let mut frame = Frame::from_bytes(assemble(0, 0x7f, false, &[1, 2, 3]).unwrap()).unwrap();
    assert_eq!(
        state.dispatch(&mut frame),
        Event::Unknown {
            extension_type: 0,
            message_type: 0x7f,
            length: 3
        }
    );
    // An extension the harness does not speak.
    let mut frame = Frame::from_bytes(assemble(2, 0x01, false, &[]).unwrap()).unwrap();
    assert!(matches!(
        state.dispatch(&mut frame),
        Event::Unknown {
            extension_type: 2,
            ..
        }
    ));
    // A known type whose payload is too short to be one.
    let mut frame = Frame::from_bytes(assemble(0, 0x01, false, &[0]).unwrap()).unwrap();
    assert!(matches!(
        state.dispatch(&mut frame),
        Event::Malformed {
            message_type: 0x01,
            length: 1,
            ..
        }
    ));
    // A message of a subprotocol the harness does not model is decoded and kept as other.
    let mut frame = Frame::from_bytes(assemble(0, 0x21, true, &[0; 36]).unwrap()).unwrap();
    assert_eq!(
        state.dispatch(&mut frame),
        Event::Other { message_type: 0x21 }
    );
    // A setup success carrying the channel bit it must not carry is decoded and flagged.
    let payload = [2, 0, 0, 0, 0, 0];
    let mut frame = Frame::from_bytes(assemble(0, 0x01, true, &payload).unwrap()).unwrap();
    assert!(matches!(
        state.dispatch(&mut frame),
        Event::SetupSuccess { .. }
    ));
    assert_eq!(state.header_issues.len(), 1);
    assert_eq!(state.events.len(), 5);
}

/// The response the runner records for a setup is what the dispatcher settled on.
#[test]
fn the_runner_reads_the_answer_from_the_dispatcher() {
    let deployment = SimulatedDeployment::new(
        1,
        vec![RoleConfig::new(WireProtocol::TemplateDistributionProtocol)],
    );
    let compiled = Compiler::new().compile(&two_setups()).unwrap();
    let execution = runner::run(&deployment, &compiled);
    let state = &execution.connections[&0];
    let SetupState::Rejected { error_code, .. } = &state.setup else {
        panic!("{:?}", state.setup);
    };
    assert_eq!(
        execution.sessions[&0].response,
        SetupResponse::Error {
            flags: 0,
            error_code: error_code.clone()
        }
    );
}
