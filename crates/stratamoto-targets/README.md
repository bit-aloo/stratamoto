# stratamoto-targets

The real roles: sv2-apps' pool, running as its own process against a real Bitcoin Core node.

## What it starts

`Node` launches Bitcoin Core with IPC enabled, reusing sv2-apps' own launcher rather than
reimplementing it. `PoolDeployment` then starts the pool binary sv2-apps builds, with a
configuration written for it that points at the node's data directory: the pool carries
`bitcoin-core-sv2` and takes its templates over the node's IPC socket, with no `sv2-tp` in
between. It implements the `Deployment` trait so the runner and the oracles apply to it
unchanged.

The pool is a separate process rather than a library linked in, the way a snapshotting fuzzer
runs a target: its crashes are its own, a crash handler can be preloaded into it, and coverage
instrumentation built into it reads its environment when it starts.

Templates come from a node rather than from the harness, so the pool is the real thing when it
is asked to set a connection up.

```rust
use stratamoto_targets::pool::PoolDeployment;

let deployment = PoolDeployment::start()?;                    // the node, then the pool
let deployment = PoolDeployment::start_with(Path::new("pool_sv2"))?; // a given binary
deployment.node().generate_blocks(1);                          // move the chain
```

`start` runs the binary `STRATAMOTO_POOL` names. The pool's log is written next to its
configuration, at `log_path()`.

Starting one waits until the pool has taken its first template and is accepting connections:
about three seconds for the node and the pool together. The pool remembers what
earlier connections did, so a run against a reused pool can see what the runs before it left
behind; a process that wants every program to start alike runs one program per pool.

## Prerequisites

The pool binary, built from sv2-apps at the revision this workspace pins:

```sh
cargo install --git https://github.com/stratum-mining/sv2-apps.git \
    --rev ab8f30f1784ea20c2de1b2726c47e7eea10f4556 pool_sv2 --root ./sv2
export STRATAMOTO_POOL=$PWD/sv2/bin/pool_sv2
```

Bitcoin Core, at a version whose IPC interface the pool speaks. sv2-apps' launcher resolves it
from a `template-provider` directory beside the working directory and downloads it when
missing, which works but is slow the first time. To reuse a copy you already have:

```sh
export STRATAMOTO_TEMPLATE_PROVIDER_CACHE=/path/to/sv2-apps/integration-tests/template-provider
```

A checkout of sv2-apps beside this one is found automatically.

The node runs in regtest.

## Tests

```sh
cargo test -p stratamoto-targets              # conformance against the real pool
cargo test -p stratamoto-targets -- --ignored # the known upstream livelock
```

| test | what it establishes |
| --- | --- |
| `pool.rs` | the pool answers `SetupConnection` within the specification, and a pool that exited is reported dead |

The ignored test in `pool.rs` asserts an unfixed sv2-apps livelock is present, so it fails once
that is fixed. See the [root README](../../README.md#a-known-upstream-finding).
