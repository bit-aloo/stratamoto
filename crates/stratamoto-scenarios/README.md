# stratamoto-scenarios

One binary per scenario. A scenario pairs a deployment with the oracles that judge it, and reads
a program on stdin.

| binary | what it runs against |
| --- | --- |
| `setup_connection` | the mock roles, on the deterministic simulator |
| `pool_setup_connection` | sv2-apps' pool, against a real Bitcoin Core node |

```sh
stratamoto generate 7 3 | STRATAMOTO_SEED=7 setup_connection
stratamoto generate 7 1 | pool_setup_connection
```

The exit code is the verdict: zero when every oracle passed, non-zero when one found a violation,
which it prints. A program that does not describe a runnable test case is skipped rather than
failed.

Generate with as many roles as the deployment has. The simulated deployment serves three, one per
subprotocol; the pool deployment serves one.

## Which oracles run where

Both scenarios check `SetupConnectionOracle` and `CrashOracle`. Only the pool scenario adds
`MiningChannelOracle`: the mock roles do not implement mining, and an unanswered channel open
says something about a pool and nothing about a role that was never asked to serve one.

## As a library

The scenarios are also a library, so the fuzzer can run them in process rather than as
subprocesses — the simulator is deterministic and needs no snapshotting:

```rust
use stratamoto_scenarios::setup_connection::{SetupConnectionScenario, TestCase, signature};

let (execution, result) = scenario.execute(&testcase);
let behaviour = signature(&execution);
```

`signature` summarizes what a run did — the set of distinct interactions it produced, not how
many of each — which is what the fuzzer uses to tell a new behaviour from a repeat.
