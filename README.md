# Lattice

A spreadsheet with a real recalculation engine, and an iced front-end that stays
smooth on a million rows.

---

## The Goal

Lattice is a spreadsheet — the thing you already know how to use — built to answer
two engineering questions that most spreadsheets quietly avoid.

**First, on the engine:** can recalculation be *incremental in the work it does*
rather than only in how long it takes? Editing one cell in a sheet of 100,000
should cost the work of editing one cell, not the work of a smaller fraction of
100,000. That means a real dependency graph — direct references *and* ranges —
that tracks exactly which cells are stale and recomputes exactly those.

**Second, on the front-end:** can a desktop canvas draw a 1,048,576-row grid where
the cost of a frame is proportional to the *window*, not to the sheet? A
spreadsheet that stutters when someone presses `Ctrl`+`↓` is not a spreadsheet.

Both questions have to be answered without cheating on correctness. It is easy to
be fast if you are allowed to be wrong, and easy to be right if you are allowed to
be slow. The interesting version is: a full evaluator — ranges, cycles,
errors-as-values, parallel recalculation — that is also cheap.

<!--
SNAPSHOT (hero, PNG): the whole window, ~1180×760, on a "garden plan" workbook.
Want in frame: a small block of filled cells, a selected range with the
translucent green wash visible, the formula bar showing `=SUM(C2:C4)` for the
active cell, and the status bar reading something like "sum 40 · average 13.3".
This is the one image that has to explain the product in one look.
-->

The shape of the thing:

```
text ──logos──▶ tokens ──chumsky──▶ AST ──compile──▶ flat RPN ──eval──▶ Value
                                                         │
                          dependency graph ──▶ dirty set ──▶ levels ──▶ rayon
```

`engine/` is the engine: cell store, dependency graph, formula language, evaluator.
It has no UI dependencies and is tested in isolation. `app/` is the iced desktop
front-end and contains no spreadsheet logic. If the engine is the ledger, the app
is the desk it sits on.

---

## What It Needs

The engine was written first and against a fixed contract, because every one of
these constraints is much cheaper to honour from the beginning than to retrofit.

**The engine must be separable.** No UI dependency, no globals, no I/O in the
logic. This is what makes the performance numbers below meaningful — they are
measured on the engine alone, with no window open.

**Storage must be sparse.** An unused cell costs nothing. Three values in a
million-row grid store three entries — not a million empty ones.

**One numeric model, honestly chosen.** IEEE-754 `f64`, as Excel and Sheets use.
The formula language is general arithmetic, not an accounting ledger: a decimal
type would either fail or silently round on division, roots and the statistical
functions a spreadsheet grows into.

**Errors are values, not crashes.** A bad cell must not abort a recalculation, and
it must not stop the cells that *depend* on it from being evaluated. Errors
propagate through the same machinery as numbers.

**Cycles must be detected, never hung on.** A circular reference is a thing a user
can type by accident, so it has to be a result — not a freeze, not a stack
overflow.

**Correctness before speed, but speed is a requirement.** Every optimization below
is guarded by a test that asserts the *work done*, not the wall-clock time, so the
guarantees survive a slower machine.

**The formula language it must support (v1):** arithmetic with precedence and
parentheses, `A1` references, `A1:B10` ranges, `$A$1` anchoring that shifts
correctly on copy and fill, and `SUM` `AVERAGE` `COUNT` `IF` `MIN` `MAX` `CONCAT`.
Plus positional parse errors — an error that points at the exact character, not
just at "the formula".

**Out of scope, deliberately:** `.xlsx` interoperability (the one optional part of
the brief), multiple sheets, cell formatting, undo/redo. See
[Known limitations](#known-limitations).

---

## The Approach

### The pipeline, in fixed stages

Each stage is a separate module with its own tests, and the boundaries are where
the bugs are *findable*.

| Stage | Crate | File | Notes |
| --- | --- | --- | --- |
| Lex | `logos` | `engine/src/lexer.rs` | every token carries its byte span |
| Parse | `chumsky 0.13` | `engine/src/parser.rs` | AST with spans; errors point at exact characters |
| Compile | — | `engine/src/compile.rs` | flat postfix bytecode with jumps |
| Evaluate | — | `engine/src/eval.rs` | stack machine, no recursion |
| Schedule | `petgraph` | `engine/src/graph.rs`, `engine/src/sheet.rs` | dirty set → levels → parallel batches |

Cells live in a `HashMap<CellRef, Cell>`. `CellRef` is a `(row, col)` pair of
`u32`s, 0-based internally and rendered 1-based; `Ref`, as written in a formula,
additionally carries a `$` flag per axis, and those flags matter only when a
formula is copied or filled. The `.xlsx`-style limits (`MAX_ROWS = 1_048_576`,
`MAX_COLS = 16_384`) are bounds for reference validation, never for allocation.

### Non-finite results never reach a cell

`x / 0` is `#DIV/0!`; any other operation producing `NaN` or `±inf` (overflow,
`(-8)^0.5`) becomes `#NUM!`. A cell therefore only ever holds a finite `f64`. The
familiar binary-float artefact (`0.1 + 0.2`) is handled where it actually matters —
in *display*: `format_number` renders at most 15 significant digits, and the grid
rounds further if a number does not fit its column.

| Value | Meaning |
| --- | --- |
| `#DIV/0!` | division by zero |
| `#REF!` | reference pushed off the sheet (a fill off the edge) |
| `#VALUE!` | operand of the wrong type |
| `#NAME?` | unknown function or name |
| `#NUM!` | numerically invalid (overflow, non-real result) |
| `#N/A` | value not available |
| `#CYCLE!` | circular reference |
| `#PARSE!` | the formula could not be parsed or compiled |

### Bytecode instead of tree walking

A formula is parsed and compiled **once**, into a flat postfix program (`Op::Const`,
`Op::Ref`, `Op::Range`, `Op::Binary`, `Op::Call { func, argc }`, `Op::Jump`,
`Op::JumpIfFalse`). Evaluation is then a loop over a `Vec`, and — more importantly —
`IF` compiles to branches, so the untaken side is never evaluated:

```
=IF(A1=0, 0, 1/A1)     with A1 = 0     →  0, not #DIV/0!
```

Every other function is strict; `IF` is the only short-circuiting form in v1.

Ranges are pushed as *range operands* and consumed by the aggregate functions, so
`SUM(A1:A1000)` never materialises a thousand intermediate values. A range used
where a scalar is expected falls back to **implicit intersection** — the formula's
row for a one-column range, its column for a one-row range, its own cell for a
rectangle containing it, otherwise `#VALUE!` — which is what spreadsheets do.

### Ranges are watched, never expanded

This is the decision the whole performance story rests on.

A formula reading a range is recorded as a single *watch* `(range, formula)`. The
graph holds edges only for direct cell references.

The tempting alternative — expanding `SUM(A1:A100000)` into a hundred thousand
edges — is quadratic in disguise, because editing any cell inside the range
rewrites that entire edge set, and `StableGraph::remove_edge` is `O(degree)`. That
theory took 8.4 seconds to become concrete on a 100k-row sheet. With watches, the
same edit touches two cells.

The two questions the scheduler actually asks are answered straight from the watch
list:

* *who reads this cell?* → every watch whose range contains it;
* *what must this formula wait for?* → the dirty cells that watch list points at.

### Incremental recalculation, in five steps

Editing a cell runs `Sheet::apply_input` then `Sheet::recalculate`:

1. **detach** — drop the edited cell's previous precedents (edges and watches);
2. **store** — write the new input; a formula starts out unevaluated;
3. **attach** — rebuild edges from the compiled program's precedents;
4. **mark** — take the transitive closure of dependents: the *dirty* set;
5. **schedule** — levelise the dirty set and evaluate each level.

Levelisation is Kahn's algorithm over the subgraph *induced by the dirty set*, so a
precedent that is already up to date imposes no ordering and no recomputation.
Cells that become ready together are, by definition, mutually independent — which
is exactly the condition for evaluating them in parallel. `rayon` runs any level
with more than one cell, and results are collected before being written back, so
the sheet is only ever read while a level runs.

`RecalcReport` returns the dirty set, the levels actually run, and any cycles — so
the guarantees are *assertable* ("editing one cell of a 100k-row sheet recomputes
exactly 2 cells") rather than merely claimed.

### Cycles: detected, not survived

A circular reference is reported as `#CYCLE!` on the cells that are *on* the cycle.
Cells merely downstream of one are still evaluated — they read the cycle error as
an ordinary value.

Cycles are found with Tarjan's algorithm, run iteratively, because a long
dependency chain must not overflow the stack. The first implementation instead
peeled dependency "ends" — which misclassifies a downstream cell that itself has
dependents. A property test caught it, and the reasoning is worth stating plainly:
peeling finds cells with no *precedents* left, but a cell on a cycle can still have
dependents, so "unresolved and has dependents" is not the same question as "is on a
cycle".

### Copy and fill

Filling is parse → shift → print → recompile (`Expr::shifted`). Relative components
move, `$` components do not, and a reference pushed off the sheet becomes `#REF!`
rather than silently pointing somewhere wrong. The AST's `Display` is a canonical
printer emitting the *minimum* parentheses needed to re-parse to the same tree,
which is what makes the round trip safe.

Literals are copied verbatim — there is no series detection, so copying `1, 2` does
not produce `3, 4`. That is deliberate v1 behaviour, not an oversight.

### The front-end

Three modules, and the split is by *kind of question*:

* `app/src/application.rs` — state, messages, update loop, widget tree.
* `app/src/grid.rs` — the canvas: viewport maths, drawing, pointer reporting.
* `app/src/theme.rs` — the palette and the chrome styles.

**Virtualisation.** The canvas draws only the rows and columns intersecting the
viewport, plus one cell of buffer. Scrolling, drawing and hit-testing are pure
functions of the scroll offset and viewport size, so a frame costs what the *window*
costs, not what the sheet costs. There is no per-cell allocation while drawing, and
the engine is only asked for values inside the visible range.

**Themes.** Warm whitewash and cream (`#FAF8F3`, `#F5F1E8`), soft sage structure
lines (`#D8DFD3`) that read as trellis rather than grid, and a narrow range of
low-contrast greens (sage `#9BAF93` → moss `#7D9471` → leaf `#5C7F52`) for the
active cell, fill handle, gutters and buttons. The selection is a translucent green
wash, not a hard box. Text is warm charcoal (`#3E3A34`), errors are muted clay
(`#9C5F4E`), numbers are right-aligned and everything else is left-aligned.

**Naming.** There is no native file picker — iced has no cross-platform dialog that
would not pull in a toolkit dependency — so a workbook is *named* rather than
browsed for. The first save opens a prompt, the file lands as `<name>.json` beside
the running program, and the open prompt lists the workbooks already in the folder
so reopening one is a single click. A name is a file name, so characters that would
steer the write out of the folder (`/`, `\`, `..`) are folded to a dash.

**Interaction.** While no editor is focused the keyboard drives the grid; typing
starts an edit, which focuses the in-cell editor and re-routes the keyboard to it.
`Escape` is the one key forwarded past a focused editor, so an edit can always be
abandoned.

| Action | Result |
| --- | --- |
| Click / drag | select a cell, or a range |
| Click a gutter | select the whole row or column |
| Double-click, `F2`, or just type | edit in place (formula bar mirrors it) |
| `Enter` / `Tab` | commit and step down / right (`Shift` reverses) |
| Arrows / `PageUp` / `PageDown` / `Home` / `End` | navigate |
| `Ctrl`+arrows | jump to the edge of the used range |
| `Shift`+arrows | extend the selection |
| `Delete` | clear everything in the selection |
| Drag the corner handle | fill, rewriting relative references |
| `Ctrl`+`S` / `Ctrl`+`Shift`+`S` | save / save under a new name |
| `Ctrl`+`O` | open a saved workbook |

The status bar doubles as an instrument panel: it shows the parse error for the
active cell, the sum and average of a selected range, or what the last
recalculation did ("filled 7 cells (recalculated 7 in 2 levels)").

<!--
SNAPSHOT (GIF, ~8s loop): the interaction that makes this feel like a spreadsheet.
Suggested beats: click C2 → type `=SUM(` → autocomplete nothing, keep typing
`C3:C6)` → Enter → the value appears and the status bar shows the recalc report →
drag the fill handle down two rows and watch the relative references rewrite.
Recording this is better than any static image because the *responsiveness* is
part of the claim. 12–15 fps, cropped to the grid plus formula bar.
-->

---

## Results

### The test suite

**195 tests**: 136 in the engine, 42 in the app, 12 property tests, 5 scale tests.

* **Unit tests** per module, including the geometry (`Metrics`) and the app's
  update loop — both pure functions that need no window.
* **Property tests** (`engine/tests/properties.rs`) for the invariants examples
  cannot cover: parse/print round trips, shape-preserving shifts, parse safety on
  arbitrary input, postfix evaluation agreeing *bit for bit* with an independent
  tree evaluator, range aggregation matching explicit addition, and the engine's
  central promise — that recalculating again changes nothing.
* **Scale tests** (`engine/tests/scale.rs`) assert **work done, not wall-clock**: an
  edit among 100k rows recomputes exactly two cells; a 20k-cell chain is linear; a
  whole-column range adds no edges.

### Performance

`cargo run --release -p engine --example recalc_bench` — 100,000 populated rows plus
a whole-column aggregate:

| Operation | Time |
| --- | --- |
| fill 100,000 cells | 45 ms |
| add `=SUM(A1:A100000)` | 1.9 ms |
| edit one cell in the middle | **1.2 ms** (2 cells recomputed) |
| full recalculation (100,001 cells) | 54 ms |
| serialise to JSON (9.2 MB) | 27 ms |

The edit is the number that matters. It is **constant work** — not "fast enough on
this machine", but a quantity that does not grow with the sheet, because ranges are
watches rather than edges and because only the dirty subgraph is touched.

<!--
SNAPSHOT (PNG, terminal): the raw `recalc_bench` output, cropped tight, in a
monospace font. Cheap to make, and it is the most honest-looking artifact in the
whole repo — an actual measurement rather than a claim in a table. Put it directly
under the table above.
-->

### Design alternatives, measured

Where a decision was genuinely contested, it was settled by measurement rather than
taste:

| Approach | Outcome |
| --- | --- |
| Expand ranges into graph edges | 8.4 s per edit on 100k rows |
| **Watch ranges instead** | **1.2 ms per edit** |
| Peel dependency "ends" to find cycles | misclassified downstream cells as cyclic |
| **Tarjan SCC, iterative** | **correct, and safe on long chains** |
| Recompute the dirty set's *closure* from scratch | correct but recomputes up-to-date cells |
| **Kahn levelisation over the induced subgraph** | **recomputes only what is stale, and exposes parallelism for free** |
| Edit-in-place hijacking: move selection on click while editing | typed text silently follows the click |
| **Settle the edit before moving the selection** | **text stays in the cell it was typed in** |

---

## What Actually Works

Everything in the "Results" table traces back to two decisions, and it is worth
being explicit about which ones carried the weight — because the obvious
optimizations did not.

**Ranges as watches is the whole thing.** The 7,000× gap between 8.4 s and 1.2 ms
is not a constant-factor tuning win; it is a change in the *shape* of the work, from
"rewrite a hundred thousand edges per edit" to "touching two cells". Everything else
— the bytecode, the leveliser, the parallel batches — is a smaller multiple on top
of it. Had this been modelled the intuitive way, no amount of profiling would have
saved it.

**Levelising the dirty subgraph, not the sheet.** Because Kahn is run only over the
cells known to be stale, "already up to date" imposes no ordering work, and cells
that come ready together are provably independent — so parallelism falls out of the
algorithm rather than being bolted on. The dirty *closure* does the reaching; the
leveliser only orders.

**And the property tests are what made it trustworthy.** Every performance win above
was verified by a test that asserts the *work*, not the time, so the guarantee
survives being run on a slower machine or in CI. More importantly, the property
tests found bugs that example-based tests structurally could not:

| Bug | Found by |
| --- | --- |
| `#DIV/0!` swallowed a following `/` (`#DIV/0!/2` lexed as one token) | parse/print round trip |
| `Graph::remove_node` swapping the last node in, invalidating stored indices | graph pruning test |
| Cells downstream of a cycle misreported as part of it | recalc-idempotence property |
| Quadratic range-edge materialisation (8.4 s per edit on 100k rows) | scale test |
| Quadratic leveliser (43 s for a 20k-cell chain) | scale test |
| Pointer hit-testing offset by the chrome height | driving the GUI with synthetic X input |
| A click during an edit moving the text to the clicked cell | driving the GUI with synthetic X input |

That last pair is the most instructive. `iced`'s `Cursor::position_over` returns
*window* coordinates filtered by the widget's bounds, so the canvas origin has to be
subtracted explicitly — a unit test could never have seen it, but clicking a cell in
a real window could. The same tooling found a second, worse bug: clicking a
different cell *while editing* moved the selection before committing, and since a
commit writes to the active cell, the text followed the click. Both were found by
running the actual binary on a virtual display and driving it with synthetic input,
which is the closest thing to "a person using it" that is available in automation.

<!--
SNAPSHOT (PNG, cropped small): the parse-error caret diagram living in the status
bar for a half-typed formula, e.g. `=SUM(C2:C4` with the caret under the end of the
span. This is a screenshot of a *failure* behaving well, which is more persuasive
than another screenshot of the happy path. Monospace font matters — capture at 1:1,
do not scale.
-->

**What did *not* matter, for the record.** The choice of `f64` over a decimal type
never showed up in any benchmark — it is a correctness/scope decision, defended in
[the numeric model](#non-finite-results-never-reach-a-cell), not a performance one.
`.xlsx` support was cut and nothing measurable got worse. Reducing allocations in
the evaluator was tried and reverted: inside the noise floor, and it made `eval.rs`
harder to read.

---

## Conclusion

**Yes, on both questions.**

The engine recalculates incrementally in work done, not just in time: editing one
cell among 100,000 costs **1.2 ms** and recomputes exactly **two cells**. The
mechanism is a dependency graph where ranges are *watched* rather than expanded into
edges — the difference between an 8.4-second edit and a 1.2-millisecond one, which
is a change of shape rather than of constant factor. On top of that, Kahn
levelisation over the dirty subgraph makes parallelism a property of the algorithm
rather than an add-on.

The front-end holds up its end: the canvas is virtualised, so a frame costs what the
window costs, and a 1,048,576-row grid scrolls like a small one.

What is genuinely satisfying is that the correctness story and the performance story
turned out to be the *same* story. Ranges-as-watches is faster *because* it is a
more honest model of what a formula actually depends on — one relationship, not a
hundred thousand. The property tests then made that model trustworthy, catching
four real bugs that example-based tests could not have reached, including a cycle
misclassification that only appears when a downstream cell has dependents of its
own.

The remaining work is scope, not capability: `.xlsx` interoperability, multiple
sheets, formatting and undo/redo are all absent, and the README says so plainly
rather than implying otherwise.

<!--
SNAPSHOT (GIF, ~5s): scrolling. Home → Ctrl+↓ on a sheet with data → the name box
showing a six-digit row number. This is the cheapest way to convey "a million rows"
without asking anyone to read a table. Alternative if GIFs feel heavy: a static
screenshot with the name box reading `A1048576`, which is less convincing but costs
nothing.
-->

---

## Resources

- `rustc` 1.82+ (edition 2021, resolver 2)
- `logos` 0.16 — lexing
- `chumsky` 0.13 — parsing
- `petgraph` 0.8 — the dependency graph (`StableGraph`)
- `rayon` 1.12 — per-level parallelism
- `serde` + `serde_json` 1 — the file format
- `thiserror` 2 — error types
- `iced` 0.14 — the GUI (`wgpu`, `canvas`, `x11`, `wayland`)
- `proptest` 1 — property tests

`tiny-skia`, iced's CPU renderer, is deliberately excluded: its `softbuffer`
dependency does not currently compile on this toolchain.

---

## Build

```sh
./run.sh                          # the spreadsheet (NixOS: sets up the X11/Vulkan loader path)
./run.sh --debug                  # debug build instead
cargo test --workspace            # all 195 tests
cargo clippy --workspace --all-targets
cargo run --release -p engine --example recalc_bench    # the performance numbers above
```

`run.sh` exists for a specific NixOS reason worth knowing: iced and wgpu `dlopen`
libraries by soname (`libX11.so.6`), and NixOS keeps them out of the default loader
path, so launching the binary directly panics with `opening library failed`. The
script assembles `LD_LIBRARY_PATH` and points the Vulkan loader at mesa's
lavapipe ICD. On other distributions, `cargo run --release -p app` is enough.

---

## Known limitations

* Single sheet per workbook; no cell formatting, borders, or column widths.
* No named ranges, absolute-sheet references, or cross-sheet formulas.
* No `AND`/`OR`/`NOT`, no short-circuiting beyond `IF`, no text functions beyond
  `CONCAT`.
* Text is never coerced to a number on entry (`"5"` stays text); numbers are never
  rendered as text.
* Series detection on fill (turning `1, 2` into `3, 4`) is deliberately absent.
* Column widths are fixed; long text is truncated with an ellipsis and a long
  number is rounded to fit.
* `.xlsx` interoperability is not implemented — the one optional part of the brief.
* No undo/redo, and no native file picker (see [Naming](#the-front-end)).

### Where the next hour would go

1. **Undo/redo.** `Sheet` is already `Clone` and every edit already returns its
   dirty set, so a snapshot-per-edit stack is a small step from here.
2. **Per-column widths.** The drawing code already computes cell rectangles in one
   place (`Metrics`), so this is a data change rather than a layout rewrite.
3. **`calamine` / `rust_xlsxwriter` round trip**, to make the format claim complete.

---

## Appendix

### Formula language (v1)

```
formula        := "="? comparison
comparison     := additive (("="|"<>"|"<"|"<="|">"|">=") additive)*
additive       := multiplicative (("+"|"-") multiplicative)*
multiplicative := power (("*"|"/") power)*
power          := unary ("^" unary)*                 left-associative
unary          := ("-"|"+") unary | postfix
postfix        := primary "%"*
primary        := number | string | error | reference | range | call | name | "(" comparison ")"
reference      := CELLREF (":" CELLREF)?
call           := IDENT "(" (comparison ("," comparison)*)? ")"
```

* **References** adjust on copy/fill per `$`; ranges are rectangular and normalised.
* **Numbers**: `1`, `1.5`, `.5`, `2e3`. **Strings**: `"abc"`, `""` escapes a quote.
* **Booleans**: `TRUE` / `FALSE` (case-insensitive). **Errors** may be typed
  literally (`=#N/A`).
* **Functions**: `SUM`, `AVERAGE`, `COUNT`, `IF`, `MIN`, `MAX`, `CONCAT` — all
  case-insensitive. Adding one is a match arm in `functions.rs`.
* `-2^2` is `4`: unary minus binds tighter than `^`, as in Excel. `^` is
  left-associative.
* Function arguments follow spreadsheet convention: a *scalar* argument coerces
  (`SUM(TRUE, 2)` is 3, `SUM("x")` is `#VALUE!`) while text, booleans and blanks
  *inside a range* are ignored.

Since tokens carry spans, the parser reports byte offsets into the original text and
`Diagnostic::render` draws a caret:

```
=1 + 2)
      ^ unexpected `)`; expected end of formula
```

### File format

A native JSON format via `serde` (`engine/src/io.rs`), versioned by `FORMAT_VERSION`.
It stores **inputs only** — literals and formula *text* — never computed values, so a
file stays small, diffable and correct if the evaluator ever changes. Loading
recompiles and recalculates everything. Newer versions are rejected rather than
misread, and out-of-range records are skipped rather than panicking.

```json
{
  "version": 1,
  "name": "garden plan",
  "cells": [
    { "row": 1, "col": 2, "kind": "number",  "value": 12.0 },
    { "row": 4, "col": 2, "kind": "formula", "source": "=SUM(C2:C4)" }
  ]
}
```

A workbook carries its own `name`, so reopening a file restores the title and the
next save goes back to the same place. The name is omitted while it is untitled.

### Where each module lives

| File | Responsibility |
| --- | --- |
| `engine/src/addr.rs` | `CellRef`, `Ref`, `RangeRef`, `Bounds`, sheet limits, A1 rendering |
| `engine/src/error.rs` | `ErrorKind`, `Diagnostic` and its caret rendering |
| `engine/src/value.rs` | `Value`, coercion, comparison, 15-significant-digit formatting |
| `engine/src/lexer.rs` | `logos` tokens with spans |
| `engine/src/ast.rs` | spanned `Expr`, precedence, `shifted()`, canonical printer |
| `engine/src/parser.rs` | the `chumsky` grammar and positional diagnostics |
| `engine/src/compile.rs` | AST → RPN bytecode, constant pool, precedents |
| `engine/src/eval.rs` | the stack machine and implicit intersection |
| `engine/src/functions.rs` | `SUM` `AVERAGE` `COUNT` `IF` `MIN` `MAX` `CONCAT` |
| `engine/src/graph.rs` | `DepGraph`, range watches, Kahn levelisation, Tarjan SCC |
| `engine/src/sheet.rs` | sparse store, dirty closure, recalculation, `ValueSource` |
| `engine/src/io.rs` | the versioned JSON workbook format |
| `app/src/application.rs` | state, messages, update loop, widget tree |
| `app/src/grid.rs` | the virtualised canvas |
| `app/src/theme.rs` | palette and chrome styles |
| `run.sh` | the NixOS launcher (X11 / Vulkan loader path) |
