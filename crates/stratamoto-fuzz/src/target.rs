use stratamoto_ir::Program;

/// What running a program against a deployment produced.
pub enum Outcome {
    /// The deployment behaved. The signature summarizes what it did, so that a program which
    /// drove it somewhere new can be told from one that repeated a known path.
    Ok { signature: u64 },
    /// The program did not describe a runnable test case.
    Skip,
    /// The deployment violated a property an oracle checks.
    Fail(String),
}

/// A scenario the fuzzer can run programs against.
pub trait Target {
    fn run(&mut self, program: &Program) -> Outcome;
    fn name(&self) -> &'static str;
}
