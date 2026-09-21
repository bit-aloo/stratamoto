# stratamoto-nyx-sys

Bindings to the [Nyx](https://nyx-fuzz.com) agent: the C code a scenario uses, inside the
snapshotting VM, to take the snapshot, receive inputs, report crashes and restore.

The agent and the crash handler are [fuzzamoto](https://github.com/dergoegge/fuzzamoto)'s,
vendored under their MIT license (see `LICENSE-fuzzamoto`).

## What the build does

- Compiles the agent into the crate. If `STRATAMOTO_POOL` names an AFL instrumented pool
  binary, the agent is built for that binary's coverage map size, which it learns by running
  the binary with `AFL_DUMP_MAP_SIZE=1`; otherwise it uses the size the host supplies.
- Compiles the crash handler into a shared object, whose path is `CRASH_HANDLER`. Preloaded
  into the target with `LD_PRELOAD`, it reports aborts and failed assertions to Nyx directly.

## Use

Nothing here works outside a Nyx VM: the hypercalls fault. The scenario runner in
`stratamoto` is built against this crate only with its `nyx` feature, and a scenario built
that way is run by the fuzzer, never by hand.
