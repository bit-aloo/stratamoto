use rand::{RngExt, seq::IndexedRandom};

use crate::{
    IndexedVariable, Operation, ProgramBuilder, Protocol, Variable,
    errors::ProgramValidationError,
    generators::{Generator, setup_connection::SetupConnectionGenerator},
};

/// The regtest limit, as a little endian `U256`. A channel asking for more work than the
/// network itself requires is the ordinary case.
const REGTEST_MAX_TARGET: [u8; 32] = {
    let mut target = [0u8; 32];
    target[29] = 0xff;
    target[30] = 0xff;
    target[31] = 0x7f;
    target
};

/// User identities the pool accepts, and one it does not.
///
/// An empty identity is read as a full donation, and `sri/solo/<address>` as solo payout. A
/// malformed one draws an `OpenMiningChannel.Error`, which is a legitimate answer worth
/// reaching rather than a failure.
const USER_IDENTITIES: [&str; 3] = ["", "sri/donate", "sri/solo/not-an-address"];

/// Opens a standard mining channel on a mining session, and sometimes submits a share against
/// the job the server announces on it.
///
/// This is the fragment that reaches the mining protocol at all: everything past a channel
/// needs identifiers the server chose, so a program cannot get here by writing values down.
pub struct MiningChannelGenerator {
    /// The role a session is opened to, if the program has no mining session to reuse.
    pub role: usize,
}

impl<R: RngExt> Generator<R> for MiningChannelGenerator {
    fn generate(
        &self,
        builder: &mut ProgramBuilder,
        rng: &mut R,
    ) -> Result<(), ProgramValidationError> {
        let session = match builder.get_random_variable(rng, &Variable::Session(Protocol::Mining)) {
            Some(session) => session,
            None => {
                // A mining channel needs a mining session, so make one when there is none.
                SetupConnectionGenerator {
                    role: self.role,
                    protocol: Some(Protocol::Mining),
                }
                .generate(builder, rng)?;
                builder
                    .get_nearest_variable(&Variable::Session(Protocol::Mining))
                    .ok_or(ProgramValidationError::VariableNotDefined(0))?
            }
        };

        let request_id = one(builder.append_op(Operation::LoadRequestId(rng.random()), &[])?);
        let identity = *USER_IDENTITIES
            .choose(rng)
            .expect("USER_IDENTITIES is not empty");
        let user_identity =
            one(builder.append_op(Operation::LoadStr(identity.to_string()), &[])?);
        let hashrate = one(builder.append_op(
            Operation::LoadHashrate(1_000_000_000f32.to_bits()),
            &[],
        )?);
        let target = one(builder.append_op(Operation::LoadTarget(REGTEST_MAX_TARGET), &[])?);

        let channel = one(builder.append_op(
            Operation::OpenStandardMiningChannel,
            &[&session, &request_id, &user_identity, &hashrate, &target],
        )?);

        if rng.random_bool(0.5) {
            // The identifiers come from the channel, so the share names a job the server
            // actually announced rather than one the program invented.
            let channel_id = one(builder.append_op(Operation::ChannelIdOf, &[&channel])?);
            let job_id = one(builder.append_op(Operation::JobIdOf, &[&channel])?);
            let sequence =
                one(builder.append_op(Operation::LoadSequenceNumber(rng.random()), &[])?);
            let nonce = one(builder.append_op(Operation::LoadNonce(rng.random()), &[])?);
            let ntime = one(builder.append_op(Operation::LoadNtime(rng.random()), &[])?);
            let version =
                one(builder.append_op(Operation::LoadBlockVersion(0x2000_0000), &[])?);

            builder.append_op(
                Operation::SubmitSharesStandard,
                &[
                    &session,
                    &channel_id,
                    &sequence,
                    &job_id,
                    &nonce,
                    &ntime,
                    &version,
                ],
            )?;
        }

        Ok(())
    }
}

fn one(mut variables: Vec<IndexedVariable>) -> IndexedVariable {
    assert_eq!(variables.len(), 1, "operation produces exactly one variable");
    variables.pop().expect("checked above")
}
