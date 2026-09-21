# stratamoto

The harness: how a compiled program reaches a role, and how what comes back is judged.

## The pieces

| module | what it is |
| --- | --- |
| `transport` | the `Transport` and `Deployment` traits every target is reached through |
| `connection` | Sv2 frames over the simulated network |
| `noise` | Sv2 frames over a real socket, with the Noise handshake real roles require |
| `deployment` | the simulated deployment, roles running on the seeded runtime |
| `roles` | the mock roles, and what each accepts |
| `runner` | carries out a compiled program's actions and records what came back |
| `oracle` | judges an execution |
| `scenario` | the `Scenario` trait and the `stratamoto_main!` entry point |

## One interface, two kinds of target

`Transport` is synchronous. A real role runs on its own runtime behind a real socket and there is
nothing for the harness to do while it waits; a simulated role runs on the deterministic
executor, and its transport drives that executor for the duration of each call, which is what
keeps the roles making progress between the harness's own steps.

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
    fn new() -> Result<Self> { /* bring up what every test case shares */ }
    fn run(&mut self, testcase: TestCase) -> ScenarioResult { /* run and judge */ }
}

stratamoto_main!(MyScenario, TestCase);
```

`stratamoto_main!` reads the input from `STRATAMOTO_INPUT` or stdin and turns the result into an
exit code. Everything a run depends on travels with the input: a program carries the seed its
simulated deployment is built from, so there is no seed to read from the environment.
