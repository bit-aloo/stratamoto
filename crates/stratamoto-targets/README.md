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

let mut deployment = PoolDeployment::start()?;   // node, sv2-tp, then the pool
deployment.template_provider().generate_blocks(1);   // move the chain
deployment.restart_pool()?;                          // a pool that has served nothing
deployment.restart_all()?;                           // and a node and sv2-tp likewise
```

Starting one waits until the pool has taken its first template and is accepting connections:
about three seconds for the node, `sv2-tp` and the pool together, and about one second for the
pool alone. The pool remembers what earlier connections did, so a caller that wants every
program to start alike replaces the pool between programs; the node and `sv2-tp` keep running
through that, since it is the pool that keeps channel state. A pool that stopped serving is
replaced the same way, with a bound on how long it is given to shut down.

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
| `isolation.rs` | a program's result is the same alone, after another program, and after one that wedged the pool; and what a restart costs, printed with `--nocapture` |

The ignored test in `pool.rs` asserts an unfixed sv2-apps livelock is present, so it fails once
that is fixed. See the [root README](../../README.md#a-known-upstream-finding).
