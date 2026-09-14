use std::collections::BTreeSet;

use stratamoto::{
    runner::{self, ChannelOutcome},
    transport::Deployment,
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::Compiler,
};
use stratamoto_targets::pool::PoolDeployment;

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

/// The regtest limit as a little endian U256.
fn regtest_max_target() -> [u8; 32] {
    let mut target = [0u8; 32];
    target[29] = 0xff;
    target[30] = 0xff;
    target[31] = 0x7f;
    target
}

/// `opens` channel opens on one mining session, optionally with a frame of a reserved message
/// type injected after the first.
fn program(opens: usize, reserved_frame_after_first: bool) -> Program {
    let mut builder =
        ProgramBuilder::new(ProgramContext { num_roles: 1, num_connections: 0, seed: 0 });
    let role = one(builder.append_op(Operation::LoadRole(0), &[]).unwrap());
    let connection = one(builder.append_op(Operation::Connect, &[&role]).unwrap());
    let setup = one(builder.append_op(Operation::BeginBuildSetupConnection, &[]).unwrap());
    let setup = one(
        builder
            .append_op(
                Operation::EndBuildSetupConnection { protocol: Protocol::Mining },
                &[&setup],
            )
            .unwrap(),
    );
    let session = one(
        builder
            .append_op(
                Operation::SendSetupConnection { protocol: Protocol::Mining },
                &[&connection, &setup],
            )
            .unwrap(),
    );

    for i in 0..opens {
        let request = one(
            builder
                .append_op(Operation::LoadRequestId(100 + i as u32), &[])
                .unwrap(),
        );
        let identity = one(builder.append_op(Operation::LoadStr(String::new()), &[]).unwrap());
        let hashrate = one(
            builder
                .append_op(Operation::LoadHashrate(1_000_000_000f32.to_bits()), &[])
                .unwrap(),
        );
        let target = one(
            builder
                .append_op(Operation::LoadTarget(regtest_max_target()), &[])
                .unwrap(),
        );
        builder
            .append_op(
                Operation::OpenStandardMiningChannel,
                &[&session, &request, &identity, &hashrate, &target],
            )
            .unwrap();

        if i == 0 && reserved_frame_after_first {
            let bytes = one(builder.append_op(Operation::LoadBytes(vec![0u8; 14]), &[]).unwrap());
            builder
                .append_op(
                    // 0x1e is reserved in the mining protocol, so the pool has no handler for it.
                    Operation::SendRawFrame { message_type: 0x1e, extension_type: 0 },
                    &[&connection, &bytes],
                )
                .unwrap();
        }
    }
    builder.finalize().unwrap()
}

fn outcomes(deployment: &PoolDeployment, program: &Program) -> Vec<ChannelOutcome> {
    let compiled = Compiler::new().compile(program).unwrap();
    let execution = runner::run(deployment, &compiled);
    let mut slots: Vec<_> = execution.channels.into_iter().collect();
    slots.sort_by_key(|(slot, _)| *slot);
    slots.into_iter().map(|(_, outcome)| outcome).collect()
}

/// Several channels on one session each get their own answer and their own identifier, and a
/// message type the pool has no handler for ends that connection without touching the pool.
///
/// The second half is why the channel oracle does not treat silence after a frame of the
/// program's own choosing as a violation: dropping a client that sends something unhandled is
/// the server's prerogative, and the pool keeps serving everyone else.
#[test]
fn opens_are_answered_until_the_client_sends_something_unhandled() {
    let deployment = PoolDeployment::start().expect("the pool starts against a real node");

    let answered = outcomes(&deployment, &program(3, false));
    let mut identifiers = BTreeSet::new();
    for (i, outcome) in answered.iter().enumerate() {
        let ChannelOutcome::Success {
            request_id,
            channel_id,
            job_id,
            ..
        } = outcome
        else {
            panic!("open {i} was not answered with a success: {outcome:?}");
        };
        assert_eq!(*request_id, 100 + i as u32, "open {i} answered the wrong request");
        assert!(job_id.is_some(), "open {i} came with no job to mine");
        assert!(identifiers.insert(*channel_id), "channel id {channel_id} was handed out twice");
    }

    let after_reserved = outcomes(&deployment, &program(3, true));
    assert!(
        matches!(after_reserved[0], ChannelOutcome::Success { .. }),
        "the open before the reserved frame should still be answered"
    );
    assert!(
        after_reserved[1..]
            .iter()
            .all(|outcome| matches!(outcome, ChannelOutcome::Silence)),
        "expected the pool to stop serving that connection: {after_reserved:?}"
    );

    // The pool itself is unharmed: a fresh connection is served as before.
    assert!(deployment.is_alive(), "the pool stopped serving entirely");
    let recovered = outcomes(&deployment, &program(3, false));
    assert!(
        recovered
            .iter()
            .all(|outcome| matches!(outcome, ChannelOutcome::Success { .. })),
        "a fresh connection was not served after the reserved frame: {recovered:?}"
    );
}
