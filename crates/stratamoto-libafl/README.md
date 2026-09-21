# stratamoto-libafl

The fuzzer: [LibAFL](https://github.com/AFLplusplus/LibAFL) clients, each driving a
[Nyx](https://nyx-fuzz.com) VM that runs a scenario binary, mutating IR programs with the
mutators, generators and minimizers of `stratamoto-ir`. A port of
[fuzzamoto-libafl](https://github.com/dergoegge/fuzzamoto/tree/master/fuzzamoto-libafl).

## How a campaign runs

1. Inside the VM, the scenario binary (built with `--features nyx`) brings the node, `sv2-tp`
   and the pool up, dumps the program context, and asks the Nyx agent for its input. That
   takes the snapshot.
2. Each client mutates a program from its corpus, hands it to the VM, and the scenario
   compiles and runs it against the pool from the snapshot, then reports: a pass, a skip, or
   a finding with the name of the oracle that made it. The VM is restored.
3. Coverage comes from the pool's AFL instrumentation through the shared map the agent set
   up. A program that reaches new coverage joins the corpus and is minimized first: cut,
   block by block, then instruction by instruction, keeping what still covers the same.
4. A finding is filed by its cause under the client's `crashes` directory: `setupconnection`,
   `crash`, `infrastructure`, or `timeout` once a hang is confirmed at a multiple of the
   timeout. Corpus and crash files are bare programs: a scenario binary replays one, and
   `stratamoto print` shows it.

## Running

Nyx needs bare metal Linux on x86_64 with KVM, and the VMware backdoor enabled in KVM. The
share directory comes from `stratamoto init`; see the [CLI](../stratamoto-cli).

```sh
mkdir -p /tmp/in
stratamoto-libafl --input /tmp/in --output /tmp/out --share /tmp/stratamoto-share --cores 0-7
```

`--iterations`, `--timeout`, `--ignore-hangs`, `--mutators a,b` and `--swarm` shape a run;
`-r <file>` reruns one input and `-m <file>` minimizes one. Without a dumped context the
number of roles programs address comes from `--roles`.

## Building

`libafl_nyx` builds QEMU-Nyx and the Nyx packer into the target directory the first time,
which needs their build dependencies (see fuzzamoto's `Dockerfile.libafl`) and a while. The
crate is not a default member of the workspace, so `cargo build` at the root leaves it alone;
`cargo build -p stratamoto-libafl` builds it.
