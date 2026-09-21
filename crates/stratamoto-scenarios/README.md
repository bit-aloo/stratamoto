# stratamoto-scenarios

One binary per scenario. A scenario pairs a deployment with the oracles that judge it, and reads
a program on stdin: either a bare one as the CLI writes it, or an artifact the fuzzer saved.

| binary | what it runs against |
| --- | --- |
| `pool_setup_connection` | sv2-apps' pool, against a real Bitcoin Core node |

```sh
stratamoto generate 7 1 | pool_setup_connection
STRATAMOTO_INPUT=failures/failure-0.stratamoto pool_setup_connection
```

A scenario process runs one test case, so no test case sees what an earlier one left behind.

The exit code is 0 for a pass or a skip, 1 for a finding, and 2 when the harness could not run
the input at all, such as a pool that could not be brought up or replaced.

A program that does not describe a runnable test case is skipped rather than failed.

Generate with as many roles as the deployment has: the pool deployment serves one.

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
