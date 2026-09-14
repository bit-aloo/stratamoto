# stratamoto-fuzz

The campaign: pick a program from the corpus, mutate it, run it, and keep what is new.

```sh
stratamoto-fuzz [iterations] [seed] [failure directory]

stratamoto-fuzz 2000 1                                  # the mock roles
STRATAMOTO_TARGET=pool stratamoto-fuzz 60 1 ./failures  # sv2-apps' pool
```

Failures are printed as programs and, when a directory is given, written there in the form a
scenario binary reads, so any of them can be replayed exactly.

## How it runs

A program is picked, mutated a few times, and discarded if the builder would not accept the
result — cheaper than running it. What remains runs in process against the scenario.

There is no coverage from inside the roles, so the feedback is what a run did through the
protocol: a program earns a corpus slot by producing a set of interactions no other program has.
That is coarser than instrumentation but it is honest about what is observable from outside.

A failure is reduced before it is reported, by cutting, then blocks, then nopping, repeated until
it stops shrinking.

## When the target dies

Once a real role stops serving, every later run fails for the same reason and says nothing new,
and minimization is worse than useless: with the target down every candidate fails alike, so the
minimizer keeps the whole program while paying a connection timeout per run. A failure that took
the target with it is reported as it was run and ends the campaign.

Against the pool this is reached quickly, because of a known upstream livelock — see the
[root README](../../README.md#a-known-upstream-finding). Restarting the target between runs is
what would let a campaign continue past it.

## Fuzzing something else

```rust
use stratamoto_fuzz::{Fuzzer, target::{Outcome, Target}};

impl Target for MyTarget {
    fn run(&mut self, program: &Program) -> Outcome { /* Ok { signature } | Skip | Fail(reason) */ }
    fn name(&self) -> &'static str { "my-target" }
    fn is_alive(&self) -> bool { true }   // a real target answers honestly
}

let mut fuzzer = Fuzzer::new(MyTarget, rng, num_roles);
fuzzer.seed(context);
let failures = fuzzer.run(10_000);
```
