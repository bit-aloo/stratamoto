# stratamoto-ir

The programs. A test case is a sequence of typed instructions rather than a byte string, and the
types are what carry the protocol's structure.

## Why it is typed

`Variable` is a *type*, not a value. An operation declares what it consumes and produces, so the
builder can refuse an instruction whose inputs are of the wrong kind or out of scope. Two
consequences matter:

- **Relations between messages are enforced.** `Session(Mining)` is a different type from
  `Session(TemplateDistribution)`, so a `SetupConnection` finalized for one subprotocol cannot
  open a session for another. A share consumes a `ChannelId` and a `JobId`, and the only way to
  get ones the server recognises is to project them out of a `Channel` with `ChannelIdOf` and
  `JobIdOf`.
- **Identifiers the server owns are not invented.** A `Channel` stands for whatever the server
  assigned, bound when it answers. Writing an identifier down with `LoadJobId` is still
  possible, and is how the rejection paths are reached deliberately.

## The pieces

| module | what it is |
| --- | --- |
| `variable` | the types a program's values have |
| `operation` | what a program can do, and the types each operation consumes and produces |
| `instruction` | an operation plus the variables it consumes |
| `builder` | appends instructions, rejecting anything ill-typed or out of scope |
| `compiler` | lowers a program to the actions a harness carries out |
| `generators` | build valid fragments: `setup_connection`, `mining_channel`, `raw_frame` |
| `mutators` | change a program: `input`, `operation`, `concat`, `generate` |
| `minimizers` | shrink a failing program: `cutting`, `block`, `nopping` |

## Building a program

```rust
use stratamoto_ir::{Operation, ProgramBuilder, ProgramContext, Protocol};

let mut builder = ProgramBuilder::new(ProgramContext {
    num_roles: 1,
    num_connections: 0,
    seed: 0,
});

let role = builder.append_op(Operation::LoadRole(0), &[])?.remove(0);
let connection = builder.append_op(Operation::Connect, &[&role])?.remove(0);
let program = builder.finalize()?;
```

Everything that leaves `finalize` is statically valid. `Program::is_statically_valid` re-checks a
program that was built some other way, such as by a mutator.

## Mutating and minimizing

A mutator may fail to find a mutation, and may leave a program the builder would reject, so a
caller checks before running it:

```rust
use stratamoto_ir::mutators::{Mutator, input::InputMutator};

if InputMutator.mutate(&mut program, &mut rng).is_ok() && program.is_statically_valid() {
    // worth running
}
```

A minimizer proposes smaller programs and is told whether each still reproduces the failure:

```rust
use stratamoto_ir::minimizers::{Minimizer, nopping::NoppingMinimizer};

let mut minimizer = NoppingMinimizer::new(program);
while let Some(candidate) = minimizer.next() {
    if still_fails(&candidate) { minimizer.success() } else { minimizer.failure() }
}
let mut smallest = minimizer.current().clone();
smallest.remove_nops();
```

Each pass runs forward once, so an instruction can only go after whatever consumed it has gone.
Repeating the pipeline until it stops shrinking is what unwinds a chain of dead instructions.

## A note on reachability

A field no operation writes is a field no mutation can reach. `tests/coverage.rs` measures what a
campaign actually varies and fails if any field of a message is stuck at a single value; it is
what caught the endpoint and device information fields never moving at all.
