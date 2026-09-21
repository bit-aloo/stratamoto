# stratamoto

A deterministic simulator and fuzzer for [Stratum V2](https://github.com/stratum-mining/sv2-spec)
roles, built along the lines of [fuzzamoto](https://github.com/oss-garage/fuzzamoto).

How far each protocol message is taken is tracked in [`FEATURES.md`](FEATURES.md).

Test cases are not byte strings. They are programs in a typed intermediate
representation, where the types record how one message depends on another: a `SetupConnection`
finalized for one subprotocol cannot be sent as another, and what needs a session can only run
inside the block that runs once the server agreed to the setup. A mutator can rewrite anything
it likes and still not fabricate a relationship the protocol does not allow, which is what
separates this from feeding random bytes at a decoder.

The scope today is the `SetupConnection` message flow: every component handles that one flow
completely before the next message is added.

The same program runs against either simulated roles on a deterministic runtime, or against the
real roles from [sv2-apps](https://github.com/stratum-mining/sv2-apps) over real sockets, and
the same conformance checks apply to both.

## Layout

| crate | what it is |
| --- | --- |
| [`stratamoto-dst`](crates/stratamoto-dst) | the runtime: a seeded executor, clock, network and filesystem |
| [`stratamoto-ir`](crates/stratamoto-ir) | the programs: typed variables, operations, builder, compiler, generators, mutators, minimizers |
| [`stratamoto`](crates/stratamoto) | the harness: transports, deployments, the runner and the oracles |
| [`stratamoto-targets`](crates/stratamoto-targets) | the real roles: sv2-apps' pool, against Bitcoin Core |
| [`stratamoto-scenarios`](crates/stratamoto-scenarios) | one binary per scenario |
| [`stratamoto-fuzz`](crates/stratamoto-fuzz) | the campaign: corpus, mutation loop, minimization |
| [`stratamoto-cli`](crates/stratamoto-cli) | generating, printing and compiling programs by hand |

A run goes: a **generator** builds a program through the **builder**, which rejects anything
ill-typed; the **compiler** lowers it to actions; the **runner** carries those out against a
**deployment**; the **oracles** judge what came back.

## Getting started

```sh
cargo build
cargo test
```

The simulated side needs nothing but a Rust toolchain (built with 1.98, edition 2024). Anything
touching a real role additionally needs Bitcoin Core and `sv2-tp`; see
[running against real roles](#running-against-real-roles).

### Run one scenario

```sh
cargo build
target/debug/stratamoto generate 7 3 | target/debug/stratamoto print          # read it
target/debug/stratamoto generate 7 3 | target/debug/setup_connection
```

A scenario binary takes a serialized program on stdin and exits non-zero when an oracle finds a
violation. The program carries the seed its simulated deployment is built from, so the same
bytes replay the same run.

### Fuzz

```sh
target/debug/stratamoto-fuzz 2000 1                      # simulated roles
STRATAMOTO_TARGET=pool target/debug/stratamoto-fuzz 60 1 ./failures   # sv2-apps' pool
```

Arguments are `[iterations] [seed] [failure directory]`. Failures are reduced before they are
reported and, when a directory is given, written there as artifacts: the program, the scenario
and verdict, the campaign seed and the corpus entry it was mutated from, and the revisions of
the roles and libraries it was found against. A scenario binary replays an artifact as it is,
and `stratamoto print` shows one.

Against the pool, the pool is replaced before every run, so no run sees what earlier ones left
behind and a run that stops the pool costs one restart rather than the campaign.

The roles are compiled into the fuzzer, so the compiler's own coverage instrumentation reaches
them: built with `RUSTFLAGS="-C instrument-coverage"`, the fuzzer also keeps any program that
reached a code region no earlier one had. See [the fuzzer's README](crates/stratamoto-fuzz/README.md#coverage-from-inside-the-target).

## Running against real roles

`stratamoto-targets` starts sv2-apps' pool in process and feeds it from a real Bitcoin Core node
with `sv2-tp` in front, reusing sv2-apps' own launchers. Templates therefore come from a node,
not from us, and the pool is the real thing when it is asked to set a connection up.

Those launchers look for their binaries in a `template-provider` directory beside the working
directory and download them when they are missing. If you already have a copy, point at it and
nothing is downloaded:

```sh
export STRATAMOTO_TEMPLATE_PROVIDER_CACHE=/path/to/sv2-apps/integration-tests/template-provider
```

Without it, a checkout of sv2-apps beside this one is found automatically.

The node runs in regtest.

## A known upstream finding

The suite has one ignored test, [`a_second_frame_in_the_teardown_window_can_livelock_the_pool`](crates/stratamoto-targets/tests/pool.rs).

On a rejected `SetupConnection` the pool answers and then sleeps one second before closing the
connection, deliberately, so the error reaches the client. A second frame arriving in that
window races the teardown, and on an unlucky interleaving a pool worker spins at close to a full
core: measured, the busiest thread burns 198 of 200 jiffies over two seconds that should be
idle, while an idle pool sits at zero. The pegged worker starves the runtime and fresh
connections fail their handshake.

It is a livelock rather than a crash, and a race that fires about half the time, so the test
repeats the trigger. It is ignored because it asserts a bug is present: when sv2-apps fixes it,
the test fails, and that is the signal to update the record.

## Environment

| variable | what it does |
| --- | --- |
| `STRATAMOTO_INPUT` | read a scenario's program or artifact from a file instead of stdin |
| `STRATAMOTO_TARGET` | `simulated` (default) or `pool` |
| `STRATAMOTO_RESET` | what the pool target replaces before each run: `pool` (default), `all` for the node and `sv2-tp` too, or `none` |
| `STRATAMOTO_CORPUS` | a directory the fuzzer keeps its corpus in, and starts from next time |
| `STRATAMOTO_TEMPLATE_PROVIDER_CACHE` | where Bitcoin Core and `sv2-tp` already live |
| `RUST_LOG` | filters logging from the harness and the real roles alike, through `tracing` |
| `LLVM_PROFILE_FILE` | where an instrumented build writes its profile at exit; `/dev/null` when only the fuzzer's observer needs the counters |
