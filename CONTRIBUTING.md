# Contributing to Lattice

Thanks for taking a look. Lattice is a small project with a few rules that are not
obvious just by reading the code, so they are written down here. They are not
style preferences — each one protects something the project is actually about.

## Before you push

```sh
cargo build --workspace
cargo test --workspace
```

Both should be clean. There are two crates:

- `engine/` — cell storage, the formula language, and the recalculation engine.
- `app/` — the iced desktop front-end.

The dependency arrow only ever points one way:

```
app (iced)  ──depends on──▶  engine
```

## Rule 1: `engine` never depends on iced, or on any UI code

The engine is the part meant to be useful on its own: embeddable in other
projects, testable without opening a window, and benchmarkable from a plain
example or `cargo test`. That is only true as long as it stays free of UI
dependencies, so:

- `engine/Cargo.toml` must not list `iced`, `wgpu`, `winit`, `softbuffer`, or any
  other windowing/rendering crate — not as a normal dependency and not as a
  dev-dependency.
- Nothing in `engine/src/` may import from the `app` crate.
- Only put something in the engine if it still makes sense without a screen.
  `CellRef`, `Value`, `Sheet`, `DepGraph` and `RecalcReport` do. Selection,
  viewport, scroll position and status-bar notices do not — those live in
  `app/src/state.rs`.

You can check the rule mechanically:

```sh
cargo tree -p engine | grep -iE 'iced|wgpu|winit|softbuffer'   # must print nothing
```

If a change seems to need UI knowledge inside the engine, the answer is almost
always to pass the data in or hand the result back out, not to add the
dependency.

## Rule 2: a change to `engine/src/graph.rs` or `engine/src/sheet.rs` needs a test that asserts the *work done*

The reason this project exists is that editing one cell recomputes only the cells
that actually depend on it. That property is easy to break silently, and a
wall-clock test will not catch it: on a small sheet, recomputing everything is
still fast, and timing assertions flake on a loaded CI machine.

So any change to the dependency graph or to the sheet's recalculation path has to
come with a test that counts the work. The counters live on `RecalcReport`, which
every mutating call returns (`Sheet::set_input`, `Sheet::fill`, `Sheet::clear`):

| what you want to prove | assert on |
| --- | --- |
| only the dirty subgraph was recomputed | `report.dirty`, `report.recalculated()` |
| independent cells were batched, not serialised | `report.depth()`, `report.levels` |
| a circular reference was reported instead of hanging | `report.cycles` |
| a whole-column range did not expand into a million edges | `sheet.graph_size()` |

```rust
// Editing one cell deep in a 100,000 row column recomputes exactly two cells:
// the edited cell, and the aggregate that reads it.
let report = sheet.set_input(CellRef::new(50_000, 0), "10");
assert_eq!(report.recalculated(), 2);
assert_eq!(number(&sheet, CellRef::new(100_000, 0)), 100_009.0);
```

`engine/tests/scale.rs` is the model for this style; read it before touching the
graph. A generous elapsed-time bound is welcome *in addition*, as a tripwire
against accidental quadratic behaviour, but it must never be the only assertion.
The bug you are guarding against is "recomputed too much", not "took too long on
this particular machine".

## Rule 3: never delete `engine/tests/properties.proptest-regressions`

That file is proptest's memory, and it is checked in on purpose.

When a property test finds a failing input, proptest shrinks it to the smallest
input that still fails and appends its seed here. On every later run, the seeds in
this file are replayed *before* any new random cases are generated, so a bug that
was fixed once cannot quietly come back.

- When proptest appends a line, that line is part of the bug fix. Commit it in the
  same change as the fix.
- Do not hand-edit or reorder it; let proptest manage it by re-running the suite.
- Deleting it does not make a failing test pass — it only stops the known-bad
  inputs from being tried again, which is exactly backwards.
- A line looks like `cc <seed-hash> # shrinks to <the failing input>`. The comment
  records what went wrong and is worth reading while debugging.

The property tests themselves (`engine/tests/properties.rs`) are the safety net
for the printer/parser round trip. Fill and copy are implemented as
parse → shift → print → re-parse, so if printing and parsing ever disagree, every
fill silently corrupts formulas. Those tests are why that cannot happen
unnoticed.

## Where tests live

Tests sit next to the code they exercise:

| file | what it covers |
| --- | --- |
| `engine/src/*.rs` | unit tests in a `#[cfg(test)] mod tests` at the bottom of each module |
| `engine/tests/properties.rs` | random-input invariants, plus the checked-in regression seeds |
| `engine/tests/scale.rs` | behaviour on large sheets, asserting work done rather than time |
| `app/src/state.rs` | construction and the read-only accessors |
| `app/src/input.rs` | keyboard and pointer handling, and the update loop |
| `app/src/persistence.rs` | saving, loading, and the naming prompt |
| `app/src/settings.rs` | the settings file, and following the desktop's theme |
| `app/src/theme.rs` | the two palettes: that they stay in step, and that light is unchanged |
| `app/src/grid.rs` | hit-testing, scrolling, and what the canvas draws |
| `app/src/application.rs` | no tests — the widget tree is declarative |

Helpers shared by more than one app test module live in
`app/src/state.rs`, in a `test_support` module guarded by `#[cfg(test)]`, so they
are never compiled into the shipped binary.

## Minimum supported Rust version

The floor is stated per crate and enforced by cargo:

- `engine` — **1.88**
- `app`, and therefore the workspace — **1.90**

These numbers were determined empirically, not guessed, and the conclusion is that
1.82 is not reachable. On the current lockfile:

```sh
cargo +1.90.0 check --workspace              # passes
cargo +1.88.0 check -p engine --all-targets  # passes
cargo +1.88.0 check --workspace              # fails: ordered-float@5.5.0 needs 1.90
```

The binding constraints are `iced` 0.14 (edition 2024, `rust-version = "1.88"`) and
`ordered-float` 5.5 (`1.90`), the latter reached through `wgpu-hal`. `iced` is the
wall — going below 1.88 would mean downgrading it and porting the UI, which is a
project rather than a cleanup pass. `ordered-float` is the soft one: 5.4.0 declares
1.63 and `wgpu-hal` accepts `">=3, <6.0"`, so pinning it back does let the whole
workspace build on 1.88. We choose not to, because the floor would silently regress
on the next `cargo update`, and 1.90 on the GUI side costs nothing that 1.88 saves.
If you want to revisit that trade, do it deliberately — don't re-derive it.

If you bump a dependency that raises either number, update it in three places in
the same change: the crate's `rust-version`, the README's "Building it" section,
and this list. The `engine` floor is the one worth defending, since that crate has
no UI dependencies and stays useful to people who depend on it as a library.

## Changing behaviour

Evaluation semantics, dependency-graph behaviour, and formula semantics are the
product. Changes there are welcome, but they are never incidental: keep the diff
focused, add tests that pin the new behaviour, and explain in the pull request
what changed and why. Refactors that are meant to be behaviour-preserving should
be provable as such — the suite passing unchanged, with the same test count, is
the evidence.
