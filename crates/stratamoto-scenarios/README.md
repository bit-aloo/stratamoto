# stratamoto-scenarios

One binary per scenario. A scenario pairs a deployment with the oracles that judge it, and reads
a program on stdin, as the CLI writes one and the fuzzer keeps them.

| binary | what it runs against |
| --- | --- |
| `pool_setup_connection` | sv2-apps' pool, against a real Bitcoin Core node |

```sh
stratamoto generate 7 1 | pool_setup_connection ./pool_sv2
STRATAMOTO_INPUT=/tmp/out/cpu_000/crashes/setupconnection-0 pool_setup_connection   # STRATAMOTO_POOL names the pool
```

The pool binary is the first argument, as a Nyx share directory passes it, or `STRATAMOTO_POOL`
when there is none. A scenario process runs one test case, so no test case sees what an earlier
one left behind.

Built with `--features nyx`, the binary talks to the fuzzer through the Nyx agent instead and
runs only inside a Nyx VM: see [`stratamoto-libafl`](../stratamoto-libafl).

Once the pool is up, the scenario tells the fuzzer the context programs are written in: how
many roles there are. Under Nyx that is dumped into the fuzzer's work directory as
`ir.context`; locally it is written to the file `STRATAMOTO_DUMP_IR_CONTEXT` names, if set.

The exit code is 0 for a pass or a skip, 1 for a finding, and 2 when the harness could not run
the input at all, such as a pool that could not be brought up or replaced.

A program that does not describe a runnable test case is skipped rather than failed.

Generate with as many roles as the deployment has: the pool deployment serves one.

## Tests

`cargo test -p stratamoto-scenarios` runs the scenario binary against the pool
`STRATAMOTO_POOL` names, so it needs what [`stratamoto-targets`](../stratamoto-targets) needs.

## Which oracles run

`HarnessIntegrityOracle` first, whose failure is an infrastructure verdict, then
`SetupConnectionOracle` and `CrashOracle`.

## As a library

The scenarios are also a library, for a test that wants the execution and not only the verdict:

```rust
use stratamoto_scenarios::pool_setup_connection::PoolSetupConnectionScenario;

let run = scenario.execute(&testcase);
let (execution, result) = (run.execution, run.result);
```
