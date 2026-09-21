# stratamoto

The harness: how a compiled program reaches a role, and how what comes back is judged.

## The pieces

| module | what it is |
| --- | --- |
| `transport` | the `Transport` and `Deployment` traits every target is reached through |
| `frame` | laying out an Sv2 frame, and holding one that arrived |
| `noise` | Sv2 frames over a real socket, with the Noise handshake real roles require |
| `roles` | what a role accepts on a connection |
| `events` | one dispatcher per connection, correlating answers with requests |
| `runner` | carries out a compiled program's actions and records what came back |
| `oracle` | judges an execution |
| `scenario` | the `Scenario` trait and the `stratamoto_main!` entry point |
| `runners` | where a scenario's input comes from and its verdict goes: files and exit codes, or the Nyx agent |

## One interface

`Transport` is synchronous. A real role runs on its own runtime behind a real socket and there is
nothing for the harness to do while it waits.

`Deployment` is what a program addresses: it opens connections to roles by index, says what each
role serves, and answers whether it is still alive. Implement it and the runner and every oracle
apply unchanged.

## The oracles

| oracle | what it checks |
| --- | --- |
| `SetupConnectionOracle` | the answer to a `SetupConnection` is one the specification allows |
| `CrashOracle` | the role is still serving after the run |

These check what the specification states, not what one implementation happens to do. Section 3.5
leaves error codes to each implementation, so they are recorded and not compared. A server may
accept flags it does not act on. And an answer is only owed to the *first* `SetupConnection` on a
connection: one sent later is already the client's violation, so silence afterwards is not a
finding. Being stricter than the specification produces failures against correct roles, which
drown the real ones.

## Writing a scenario

```rust
use stratamoto::scenario::{Scenario, ScenarioResult};

impl Scenario<TestCase> for MyScenario {
    fn new(args: &[String]) -> Result<Self> { /* bring up what every test case shares */ }
    fn run(&mut self, testcase: TestCase) -> ScenarioResult { /* run and judge */ }
}

stratamoto_main!(MyScenario, TestCase);
```

`stratamoto_main!` sets the scenario up from the command line, asks the runner for the input,
runs it and reports the result through the runner. Built without features, the runner reads
`STRATAMOTO_INPUT` or stdin and reports through the exit code: 0 for a pass or a skip, 1 for a
finding, 2 when the harness could not run the input at all. Built with the `nyx` feature, the
runner is the Nyx agent: asking for the input takes the VM snapshot, a finding is reported to
the fuzzer with its message, and the snapshot is restored afterwards. A binary built that way
runs only inside a Nyx VM.

A finding's message starts with `FAIL: ` and the name of the oracle that found it; a run the
harness could not complete starts with `INFRASTRUCTURE: `. The fuzzer files findings by that
prefix.
