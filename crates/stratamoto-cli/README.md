# stratamoto-cli

Building, reading and lowering programs by hand. Installed as the `stratamoto` binary.

## Commands

```sh
stratamoto generate [seed] [roles]   # write a program for a seed to stdout
stratamoto print                     # pretty print a program, or an artifact, from stdin
stratamoto compile                   # show the actions a program lowers to
```

Programs are serialized with `postcard`, which is also what the scenario binaries read, so these
compose:

```sh
stratamoto generate 7 3 | stratamoto print
stratamoto generate 7 3 | stratamoto compile
stratamoto generate 7 1 | pool_setup_connection
stratamoto print < failures/failure-0.stratamoto
```

`print` and `compile` also take an artifact the fuzzer saved. For an artifact, `print` shows the
envelope first: the scenario, the verdict, the campaign seed and iteration, what the harness was
built against, and after the program the corpus entry it was mutated from.

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
