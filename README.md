# stratamoto

A fuzzer for [Stratum V2](https://github.com/stratum-mining/sv2-spec) roles, built along the
lines of [fuzzamoto](https://github.com/dergoegge/fuzzamoto).

How far each protocol message is taken is tracked in [`FEATURES.md`](FEATURES.md).

Test cases are not byte strings. They are programs in a typed intermediate
representation, where the types record how one message depends on another: a `SetupConnection`
finalized for one subprotocol cannot be sent as another, and what needs a session can only run
inside the block that runs once the server agreed to the setup. A mutator can rewrite anything
it likes and still not fabricate a relationship the protocol does not allow, which is what
separates this from feeding random bytes at a decoder.

The scope today is the `SetupConnection` message flow: every component handles that one flow
completely before the next message is added.

Programs run against the real roles from [sv2-apps](https://github.com/stratum-mining/sv2-apps)
over real sockets. A deterministic runtime for simulated roles is being sketched in
`stratamoto-dst`, and is not wired in yet.

## Layout

| crate | what it is |
| --- | --- |
| [`stratamoto-dst`](crates/stratamoto-dst) | a seeded executor, clock, network and filesystem, in progress and not yet used |
| [`stratamoto-ir`](crates/stratamoto-ir) | the programs: typed variables, operations, builder, compiler, generators, mutators, minimizers |
| [`stratamoto`](crates/stratamoto) | the harness: the transport, the runner and the oracles |
| [`stratamoto-targets`](crates/stratamoto-targets) | the real roles: sv2-apps' pool, against Bitcoin Core |
| [`stratamoto-scenarios`](crates/stratamoto-scenarios) | one binary per scenario |
| [`stratamoto-nyx-sys`](crates/stratamoto-nyx-sys) | the Nyx agent a scenario talks to the snapshotting VM through, vendored from fuzzamoto |
| [`stratamoto-libafl`](crates/stratamoto-libafl) | the fuzzer: LibAFL clients driving Nyx VMs, mutating programs |
| [`stratamoto-cli`](crates/stratamoto-cli) | generating, printing and compiling programs by hand, and building a Nyx share directory |

A run goes: a **generator** builds a program through the **builder**, which rejects anything
ill-typed; the **compiler** lowers it to actions; the **runner** carries those out against a
**deployment**; the **oracles** judge what came back.

A campaign goes the way fuzzamoto's does: a scenario binary boots inside a
[Nyx](https://nyx-fuzz.com) VM, brings the roles up and takes a snapshot; the fuzzer mutates
programs and runs each from that snapshot, with coverage from the pool's AFL instrumentation;
findings are filed by the oracle that made them.

## Getting started

```sh
cargo build
cargo test
```

The programs and the harness need nothing but a Rust toolchain (built with 1.98, edition 2024).
Anything touching a real role additionally needs Bitcoin Core and `sv2-tp`; see
[running against real roles](#running-against-real-roles).

### Run one scenario

```sh
cargo build
target/debug/stratamoto generate 7 1 | target/debug/stratamoto print          # read it
target/debug/stratamoto generate 7 1 | target/debug/pool_setup_connection
```

A scenario binary takes a serialized program on stdin and exits non-zero when an oracle finds a
violation.

### Fuzz

Fuzzing needs bare metal Linux on x86_64 with KVM, and the VMware backdoor enabled in KVM:

```sh
sudo modprobe -r kvm-intel kvm       # or kvm-amd
sudo modprobe kvm enable_vmware_backdoor=y && sudo modprobe kvm-intel
```

Then, with a pool binary instrumented by [cargo-afl](https://github.com/rust-fuzz/afl.rs) (an
uninstrumented one runs too, with no coverage to guide the fuzzer):

```sh
export STRATAMOTO_POOL=/path/to/instrumented/pool_sv2
cargo build --release -p stratamoto-scenarios --features nyx   # the scenario, for the VM
cargo build --release -p stratamoto-libafl                     # the fuzzer, builds QEMU-Nyx
cargo build --release -p stratamoto-cli
target/release/stratamoto init --sharedir /tmp/share \
    --scenario target/release/pool_setup_connection --pool $STRATAMOTO_POOL \
    --template-provider /path/to/sv2-apps/integration-tests/template-provider \
    --nyx-dir target/release
mkdir -p /tmp/in
target/release/stratamoto-libafl --input /tmp/in --output /tmp/out --share /tmp/share --cores 0-7
```

`STRATAMOTO_POOL` is read when the Nyx agent is built, so that the shared coverage map is the
size the pool's instrumentation expects. Findings land under `/tmp/out/cpu_*/crashes`, filed by
cause, as bare programs a scenario binary replays. See [`stratamoto-libafl`](crates/stratamoto-libafl).

## Running against real roles

`stratamoto-targets` starts sv2-apps' pool binary as its own process and feeds it from a real
Bitcoin Core node with `sv2-tp` in front, reusing sv2-apps' own launchers. Templates therefore
come from a node, not from us, and the pool is the real thing when it is asked to set a
connection up.

The pool binary is built from sv2-apps at the revision this workspace pins, and named by
`STRATAMOTO_POOL`:

```sh
cargo install --git https://github.com/stratum-mining/sv2-apps.git \
    --rev ab8f30f1784ea20c2de1b2726c47e7eea10f4556 pool_sv2 --root ./sv2
export STRATAMOTO_POOL=$PWD/sv2/bin/pool_sv2
```

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
| `STRATAMOTO_POOL` | the pool binary the pool target runs |
| `STRATAMOTO_DUMP_IR_CONTEXT` | where a locally run scenario writes the program context it dumps for the fuzzer |
| `STRATAMOTO_TEMPLATE_PROVIDER_CACHE` | where Bitcoin Core and `sv2-tp` already live |
| `RUST_LOG` | filters logging from the harness and the real roles alike, through `tracing` |
