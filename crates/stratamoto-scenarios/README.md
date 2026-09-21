# stratamoto-scenarios

One binary per scenario. A scenario pairs a deployment with the oracles that judge it, and reads
a program on stdin: either a bare one as the CLI writes it, or an artifact the fuzzer saved.

| binary | what it runs against |
| --- | --- |
| `setup_connection` | the mock roles, on the deterministic simulator |
| `pool_setup_connection` | sv2-apps' pool, against a real Bitcoin Core node, replaced before every test case |

```sh
stratamoto generate 7 3 | setup_connection
stratamoto generate 7 1 | pool_setup_connection
STRATAMOTO_INPUT=failures/failure-0.stratamoto setup_connection
```

The simulated deployment is built from the seed the program carries, so the same bytes always
run on the same deployment. The pool has no seed to take: it runs on its own runtime, and what
makes its runs alike is that the pool is replaced before each one, so no test case sees what an
earlier one left behind. `STRATAMOTO_RESET` chooses what is replaced: `pool` (the default),
`all` for the node and `sv2-tp` too, or `none`.

The exit code is 0 for a pass or a skip, 1 for a finding, and 2 when the harness could not run
the input at all, such as a pool that could not be brought up or replaced.

A program that does not describe a runnable test case is skipped rather than failed.

Generate with as many roles as the deployment has. The simulated deployment serves three, one per
subprotocol; the pool deployment serves one.

## Which oracles run

Both scenarios run `HarnessIntegrityOracle` first, whose failure is an infrastructure verdict,
then `SetupConnectionOracle` and `CrashOracle`.

## As a library

The scenarios are also a library, so the fuzzer can run them in process rather than as
subprocesses — the simulator is deterministic and needs no snapshotting:

```rust
use stratamoto_scenarios::setup_connection::{SetupConnectionScenario, TestCase};

let run = scenario.execute(&testcase);
let (execution, result, digest) = (run.execution, run.result, run.digest);
```

The digest is a normalized summary of what the run did: the classes of answers it drew, the
protocol states each connection went through, what arrived unprompted or could not be decoded,
how each action ended, and the verdict, with the identifiers the server assigned and the text
of its errors left out. It is what the fuzzer uses to tell a new behaviour from a repeat, and
it is saved with a failure's trace in its artifact.
