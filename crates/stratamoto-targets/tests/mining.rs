use stratamoto::{
    oracle::{CrashOracle, MiningChannelOracle, Oracle, OracleResult, SetupConnectionOracle},
    runner::{self, ChannelOutcome, Execution, ShareOutcome},
};
use stratamoto_ir::{
    IndexedVariable, Operation, Program, ProgramBuilder, ProgramContext, Protocol,
    compiler::Compiler,
};
use stratamoto_targets::pool::PoolDeployment;

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    variables.remove(0)
}

fn context() -> ProgramContext {
    ProgramContext {
        num_roles: 1,
        num_connections: 0,
        seed: 0,
    }
}

/// The regtest limit as a little endian U256.
fn regtest_max_target() -> [u8; 32] {
    let mut target = [0u8; 32];
    target[29] = 0xff;
    target[30] = 0xff;
    target[31] = 0x7f;
    target
}

/// A mining session, a standard channel on it, and optionally a share against the job the pool
/// announces. The share names the channel and the job the pool chose, never values written here.
fn open_channel(submit_share: bool, job: Option<u32>) -> Program {
    let mut b = ProgramBuilder::new(context());
    let role = one(b.append_op(Operation::LoadRole(0), &[]).unwrap());
    let connection = one(b.append_op(Operation::Connect, &[&role]).unwrap());

    let min = one(b.append_op(Operation::LoadVersion(2), &[]).unwrap());
    let max = one(b.append_op(Operation::LoadVersion(2), &[]).unwrap());
    let setup = one(b.append_op(Operation::BeginBuildSetupConnection, &[]).unwrap());
    b.append_op(Operation::SetVersions, &[&setup, &min, &max]).unwrap();
    let setup = one(
        b.append_op(
            Operation::EndBuildSetupConnection { protocol: Protocol::Mining },
            &[&setup],
        )
        .unwrap(),
    );
    let session = one(
        b.append_op(
            Operation::SendSetupConnection { protocol: Protocol::Mining },
            &[&connection, &setup],
        )
        .unwrap(),
    );

    let request = one(b.append_op(Operation::LoadRequestId(7), &[]).unwrap());
    let identity = one(b.append_op(Operation::LoadStr(String::new()), &[]).unwrap());
    let hashrate = one(
        b.append_op(Operation::LoadHashrate(1_000_000_000f32.to_bits()), &[])
            .unwrap(),
    );
    let target = one(b.append_op(Operation::LoadTarget(regtest_max_target()), &[]).unwrap());
    let channel = one(
        b.append_op(
            Operation::OpenStandardMiningChannel,
            &[&session, &request, &identity, &hashrate, &target],
        )
        .unwrap(),
    );

    if submit_share {
        let channel_id = one(b.append_op(Operation::ChannelIdOf, &[&channel]).unwrap());
        let job_id = match job {
            Some(id) => one(b.append_op(Operation::LoadJobId(id), &[]).unwrap()),
            None => one(b.append_op(Operation::JobIdOf, &[&channel]).unwrap()),
        };
        let sequence = one(b.append_op(Operation::LoadSequenceNumber(1), &[]).unwrap());
        let nonce = one(b.append_op(Operation::LoadNonce(0), &[]).unwrap());
        let ntime = one(b.append_op(Operation::LoadNtime(0), &[]).unwrap());
        let version = one(b.append_op(Operation::LoadBlockVersion(0x2000_0000), &[]).unwrap());
        b.append_op(
            Operation::SubmitSharesStandard,
            &[&session, &channel_id, &sequence, &job_id, &nonce, &ntime, &version],
        )
        .unwrap();
    }

    b.finalize().unwrap()
}

fn run_checked(deployment: &PoolDeployment, program: &Program) -> Execution {
    let compiled = Compiler::new().compile(program).unwrap();
    let execution = runner::run(deployment, &compiled);
    for result in [
        SetupConnectionOracle.evaluate(deployment, &compiled, &execution),
        MiningChannelOracle.evaluate(deployment, &compiled, &execution),
        CrashOracle.evaluate(deployment, &compiled, &execution),
    ] {
        if let OracleResult::Fail(reason) = result {
            panic!("{reason}");
        }
    }
    execution
}

/// The chain the typed relation exists for: a session, a channel the pool opened, a job the
/// pool built from a real template, and a share naming both.
#[test]
fn the_real_pool_opens_a_channel_and_answers_a_share() {
    let deployment = PoolDeployment::start().expect("the pool starts against a real node");

    let execution = run_checked(&deployment, &open_channel(false, None));
    let ChannelOutcome::Success {
        request_id,
        channel_id,
        job_id,
        ..
    } = execution.channels[&0].clone()
    else {
        panic!("the pool refused the channel: {:?}", execution.channels[&0]);
    };
    assert_eq!(request_id, 7, "the success answers the request it was sent");
    assert!(
        job_id.is_some(),
        "the pool announced no job for channel {channel_id}, so nothing could be mined on it"
    );

    // A share against the job the pool announced is not rejected for naming the wrong thing.
    let execution = run_checked(&deployment, &open_channel(true, None));
    let share = execution.shares.first().expect("a share was submitted");
    assert!(
        !matches!(share, ShareOutcome::Unresolved),
        "the share never reached the pool: {share:?}"
    );
    if let ShareOutcome::Error { error_code, .. } = share {
        assert_ne!(error_code, "invalid-channel-id", "share named an unopened channel");
        assert_ne!(error_code, "invalid-job-id", "share named a job the pool never announced");
    }

    // A job identifier the program made up is rejected, which is what the relation buys: only
    // an identifier taken from the channel survives this.
    let execution = run_checked(&deployment, &open_channel(true, Some(0xdead_beef)));
    let share = execution.shares.first().expect("a share was submitted");
    assert!(
        matches!(share, ShareOutcome::Error { .. }),
        "an invented job id was not rejected: {share:?}"
    );
}
