# deterministic-simulator

The runtime the simulated roles run on. Everything that would otherwise be a source of
nondeterminism — task scheduling, the clock, the network, the filesystem, randomness — comes
from here and is driven by a seed, so a run is a pure function of that seed and the program.

## What is in it

| module | what it gives you |
| --- | --- |
| `task` | an executor that runs tasks to completion in a fixed order |
| `time` | a clock that only advances when nothing is runnable, plus `sleep` and `timeout` |
| `net` | an in-memory network addressed by `SocketAddr`, with configurable latency and packet loss |
| `fs` | an in-memory filesystem |
| `rand` | a seeded generator the rest of the runtime draws from |

## Using it

```rust
use deterministic_simulator::Runtime;

let runtime = Runtime::new_with_seed(7);
let handle = runtime.local_handle("10.0.0.1:34254".parse().unwrap());

handle.spawn(async { /* a role */ }).detach();
runtime.block_on(async { /* the harness */ });
```

Each participant gets a `LocalHandle` bound to an address, carrying its own view of the network,
clock and filesystem. `block_on` may be called repeatedly: spawned tasks keep making progress
each time the queue is drained, which is how a blocking harness drives an asynchronous
deployment step by step.

Time only moves when no task can run, so a `timeout` never fires early and a test never waits in
real time. The executor panics rather than hanging if asked to advance with nothing pending,
which turns a deadlock into an immediate failure.
