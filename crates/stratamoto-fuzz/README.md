# stratamoto-fuzz

The campaign: pick a program from the corpus, mutate it, run it, and keep what is new.

```sh
stratamoto-fuzz [iterations] [seed] [failure directory]

stratamoto-fuzz 2000 1                                  # the mock roles
STRATAMOTO_TARGET=pool stratamoto-fuzz 60 1 ./failures  # sv2-apps' pool
```

Failures are printed as programs and, when a directory is given, written there as artifacts a
scenario binary reads as they are: the program, the scenario and verdict, the campaign seed and
the corpus entry the program was mutated from, and the revisions of the roles and libraries the
binary was built against. The seed drives the campaign and, for the simulated roles, the
deployment each program runs on; it is written once, inside the program.

## How it runs

A program is picked, mutated a few times, and discarded if the builder would not accept the
result — cheaper than running it. What remains runs in process against the scenario.

The feedback is what a run did through the protocol, as a normalized digest: the classes of
answers it drew, the states each connection went through, what arrived unprompted or could not
be decoded, and how each action ended, without the identifiers the server assigned. A program
earns a corpus slot by showing a fact of that digest no earlier program showed, the way a run
earns one under coverage by reaching an edge no earlier run reached; keyed by whole digests the
corpus would keep every new combination of old facts. That is coarser than instrumentation but
it is honest about what is observable from outside, and it is all there is unless the binary
is built with coverage, below.

A failure is run once more after it is reduced, so what is reported is known to reproduce, and
the trace of that run is saved in the artifact with the program.

A failure is reduced before it is reported, by cutting, then blocks, then nopping, repeated until
it stops shrinking.

## Scheduling and persistence

Entries are not picked uniformly. One whose mutants keep earning slots is picked more, so is
one that reached new code, and a short one over a long one; one picked many times for nothing
is picked less. With `STRATAMOTO_CORPUS=<dir>` every entry admitted is written to that
directory as it is, one program per file, and a later campaign started with the same
directory runs each saved program once and starts from all of them that still behave.

## Coverage from inside the target

The roles under test are compiled into this binary, so the compiler's own instrumentation
reaches them without a VM or a snapshot:

```sh
RUSTFLAGS="-C instrument-coverage" LLVM_PROFILE_FILE=/dev/null cargo run -p stratamoto-fuzz -- 2000 1
```

Built that way, every crate counts how often each of its code regions ran, in one array the
profiler runtime keeps in the process. The fuzzer's observer resets that array before each run
and reads it after, and a program that reached a region no earlier run had earns a corpus slot
whatever its behaviour signature said. The build script turns the observer on when it sees the
flag; without it the observer is absent and the signature is the only feedback.

Regions are the compiler's unit: a function, a branch arm, a loop body. The pool's own threads
bump counters between runs too, so a region reached by chance is kept once and never again,
which costs a corpus slot and nothing else. `LLVM_PROFILE_FILE=/dev/null` stops the runtime
writing a profile file at exit, which the observer does not need.

## When the target dies

The pool target replaces the pool before every run, so a run that stops the pool is a finding
about that run, reported by its crash oracle, and the next run gets a pool that has served
nothing. That is what lets a campaign continue past the known upstream livelock (see the
[root README](../../README.md#a-known-upstream-finding)) and what lets such a failure be
minimized: every candidate runs on a fresh pool, so the ones that reproduce it can be told from
the ones that do not. `STRATAMOTO_RESET=none` turns the replacement off.

A target that cannot be replaced between runs is different. Once it stops serving, every later
run fails for the same reason and says nothing new, and with the target down every minimization
candidate fails alike, so a failure that took such a target with it is reported as it was run
and ends the campaign.

## When the harness fails

A pool that cannot be brought up or replaced is not a finding: it says nothing about the program
that was about to run. The campaign stops with a verdict of its own, the exit code is 2 rather
than 1, and a minimization in progress reports what it has rather than shrink against a target
it cannot trust.

## Fuzzing something else

```rust
use stratamoto_fuzz::{Fuzzer, target::{Outcome, Target}};

impl Target for MyTarget {
    fn run(&mut self, program: &Program) -> Outcome { /* Ok { signature } | Skip | Fail(reason) */ }
    fn name(&self) -> &'static str { "my-target" }
    fn is_alive(&self) -> bool { true }   // a real target answers honestly
}

let mut fuzzer = Fuzzer::new(MyTarget, rng, num_roles);
let mut failures = fuzzer.seed(context);   // a seed that already fails is a finding too
failures.extend(fuzzer.run(10_000));
```
