# stratamoto-cli

Building, reading and lowering programs by hand. Installed as the `stratamoto` binary.

## Commands

```sh
stratamoto generate [seed] [roles]   # write a program for a seed to stdout
stratamoto print                     # pretty print a program from stdin
stratamoto compile                   # show the actions a program lowers to
stratamoto init <options>            # create a Nyx share directory for a scenario
```

Programs are serialized with `postcard`, which is also what the scenario binaries read, so these
compose:

```sh
stratamoto generate 7 3 | stratamoto print
stratamoto generate 7 3 | stratamoto compile
stratamoto generate 7 1 | pool_setup_connection
stratamoto print < /tmp/out/cpu_000/crashes/setupconnection-0
```

`roles` is how many roles the program may address and defaults to 3. It has to match the
deployment a scenario runs against: the pool deployment has one role, so generate with `1` when
the program is headed there.

## Reading the output

```
// roles=3 connections=0 seed=7
v0 <- LoadRole(0)
v1 <- Connect(v0)
v2 <- LoadVersion(2)
BeginBuildSetupConnection -> v3
  SetVersions(v3, v2, v2)
v4 <- EndBuildSetupConnection<mining>(v3)
v5 <- SendSetupConnection<mining>(v1, v4)
```

Each line is an instruction. `vN <-` names what it produces, the parenthesised arguments are the
variables it consumes, and `-> vN` after a block beginning is what that block makes available
inside itself. Indentation is a scope: a message under construction only exists between its
`Begin` and `End`.

`compile` prints each action with the instruction index it came from, which is how a failure in a
run is traced back to the line of the program responsible for it.

## `init`

A Nyx VM boots from a share directory: every file the scenario needs inside the VM, Nyx's own
helpers, the VM's configuration, and the script Nyx runs at boot. `init` builds one:

```sh
stratamoto init --sharedir /tmp/share \
    --scenario target/release/pool_setup_connection \
    --pool /path/to/pool_sv2 \
    --template-provider /path/to/sv2-apps/integration-tests/template-provider \
    --nyx-dir target/release
```

| option | what it is |
| --- | --- |
| `--sharedir` | where to create the directory; it must not exist |
| `--scenario` | the scenario binary, built with `--features nyx` |
| `--pool` | the pool binary the scenario runs, AFL instrumented for coverage |
| `--template-provider` | the directory sv2-apps' launcher keeps Bitcoin Core in |
| `--nyx-dir` | a directory holding Nyx's `packer`: AFL++'s `nyx_mode`, or the target directory `libafl_nyx` built into |
| `--crash-handler` | the handler to preload into the pool; defaults to the one `stratamoto-nyx-sys` builds |
| `--memory` | the VM's memory in megabytes; 4096 by default |

The scenario, the pool, the crash handler and every shared library any of them or the node's
binaries load are copied in, resolved with `lddtree` from pax-utils when it is installed and
`ldd` otherwise. The template provider directory's `bitcoin-*` entries go in as
one archive, unpacked in the VM where the launcher looks for them. At boot the script fetches
it all into `/tmp`, brings the loopback up, and starts the scenario with the pool behind a
proxy script that preloads the crash handler into it.
