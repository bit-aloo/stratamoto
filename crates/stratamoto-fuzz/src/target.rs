use stratamoto_ir::Program;

/// What a run did, as the facts it showed, each as one number.
///
/// A program is new to a corpus if it showed any fact no earlier program did, the way a run
/// is new to a coverage map if it reached any edge no earlier run did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Behaviour {
    pub features: Vec<u64>,
}

impl Behaviour {
    /// A behaviour that is one fact, for a target that tells runs apart by a single number.
    #[must_use]
    pub fn single(feature: u64) -> Self {
        Self {
            features: vec![feature],
        }
    }
}

/// What running a program against a deployment produced.
pub enum Outcome {
    /// The deployment behaved, and this is what it did, so that a program which drove it
    /// somewhere new can be told from one that repeated a known path.
    Ok(Behaviour),
    /// The program did not describe a runnable test case.
    Skip,
    /// The deployment violated a property an oracle checks.
    Fail(String),
    /// The harness could not give the program a run: the deployment could not be brought up or
    /// reset. It says nothing about the program, and nothing can run until it is fixed, so it
    /// ends the campaign rather than count as a finding.
    Infrastructure(String),
}

/// A scenario the fuzzer can run programs against.
pub trait Target {
    fn run(&mut self, program: &Program) -> Outcome;
    fn name(&self) -> &'static str;

    /// Whether the target can still be run against.
    ///
    /// Simulated roles cannot stop, so the default is always alive. A real one that has
    /// crashed or shut itself down answers `false`, and the campaign ends there: every later
    /// run would fail for the same reason and say nothing new. A real one that is replaced
    /// between runs answers `true` whatever the last run did to it, and a failure to replace
    /// it is reported through [`Outcome::Infrastructure`] instead.
    fn is_alive(&self) -> bool {
        true
    }

    /// The record of the last run, encoded for an artifact, if the target keeps one.
    fn trace(&self) -> Option<Vec<u8>> {
        None
    }
}
