# stratamoto-targets

The real roles: sv2-apps' pool, running in process against a real Bitcoin Core node.

## What it starts

`TemplateProvider` launches Bitcoin Core with IPC enabled and the `sv2-tp` binary in front of it,
reusing sv2-apps' own launchers rather than reimplementing them. `PoolDeployment` then starts
`PoolSv2` on its own tokio runtime, pointed at that provider, and implements the
`Deployment` trait so the runner and the oracles apply to it unchanged.

Templates come from a node rather than from the harness, so the pool is the real thing when it
is asked to set a connection up.

```rust
use stratamoto_targets::pool::PoolDeployment;

let deployment = PoolDeployment::start()?;          // node, sv2-tp, then the pool
deployment.template_provider().generate_blocks(1);   // move the chain
```

Starting one waits until the pool has taken its first template and is accepting connections:
about three seconds for the node, `sv2-tp` and the pool together. The pool remembers what
earlier connections did, so a run against a reused pool can see what the runs before it left
behind; a process that wants every program to start alike runs one program per pool.

## Prerequisites

Bitcoin Core and `sv2-tp` binaries. sv2-apps' launchers resolve them from a `template-provider`
directory beside the working directory and download them when missing, which works but is slow
the first time. To reuse a copy you already have:

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
| `pool.rs` | the pool answers `SetupConnection` within the specification |

The ignored test in `pool.rs` asserts an unfixed sv2-apps livelock is present, so it fails once
that is fixed. See the [root README](../../README.md#a-known-upstream-finding).
