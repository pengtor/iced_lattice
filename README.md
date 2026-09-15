# Lattice

A spreadsheet with a real recalculation engine, and an iced front-end that stays
smooth on a million rows.

```
┌──────────────┐   ┌───────────────────────────────────────────────────────────┐
│   A          │   │  =SUM(C2:C4)                                              │
│   B          │   │  text ──logos──▶ tokens ──chumsky──▶ AST ──compile──▶ RPN  │
│   C          │   │                                    │                      │
└──────────────┘   │                                    ▼                      │
                   │              dependency graph ──▶ dirty set ──▶ levels     │
                   └───────────────────────────────────────────────────────────┘
```

`engine/` is the engine: cell store, dependency graph, formula language, evaluator.
It has no UI dependencies and is tested in isolation. `app/` is the iced desktop
front-end and contains no spreadsheet logic.

```sh
cargo run -p app          # the spreadsheet
cargo test                # all tests: engine + app
cargo run --release -p engine --example recalc_bench   # performance numbers
```

---

## 1. Engine architecture

### Pipeline

| Stage | Crate | File | Notes |
| --- | --- | --- | --- |
| Lex | `logos` | `engine/src/lexer.rs` | every token carries its byte span |
| Parse | `chumsky 0.13` | `engine/src/parser.rs` | AST with spans; errors point at exact characters |
| Compile | — | `engine/src/compile.rs` | flat postfix bytecode with jumps |
| Evaluate | — | `engine/src/eval.rs` | stack machine, no recursion |
| Schedule | `petgraph` | `engine/src/graph.rs`, `engine/src/sheet.rs` | dirty set → levels → parallel batches |

### Sparse storage

Cells live in a `HashMap<CellRef, Cell>` (`engine/src/sheet.rs`). An unused cell
costs nothing at all: three values in a million-row grid store three entries. The
`.xlsx`-style limits (`MAX_ROWS = 1_048_576`, `MAX_COLS = 16_384`) are only bounds
for reference validation, never for allocation.

`CellRef` is a `(row, col)` pair of `u32`s, 0-based internally and rendered
1-based. `Ref`, as written in a formula, additionally carries a `$` flag per axis;
the flags matter only when a formula is copied or filled.

### Numeric model: `f64`, with errors as values

The engine uses IEEE-754 doubles, as Excel and Google Sheets do. The formula
language is general arithmetic, not an accounting ledger: a decimal type would
either fail or silently round on division, roots and the statistical functions that
a spreadsheet grows into. The familiar binary-float artefact (`0.1 + 0.2`) is
handled where it matters, in *display*: `format_number` renders at most 15
significant digits, and the grid rounds a number further if it does not fit its
column (`grid::fit_number`).

Non-finite results never reach a cell. `x / 0` is `#DIV/0!`; any other operation
producing `NaN` or `±inf` (overflow, `(-8)^0.5`) becomes `#NUM!`. A cell therefore
only ever holds a finite `f64`.

Errors are *first-class values* and propagate through the evaluator rather than
aborting a recalculation:

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

A cell's formula is parsed and compiled **once**, into a flat postfix program
(`Op::Const`, `Op::Ref`, `Op::Range`, `Op::Binary`, `Op::Call { func, argc }`,
`Op::Jump`/`Op::JumpIfFalse`). Evaluation is then a loop over a `Vec`, and — more
importantly — `IF` compiles to branches, so the untaken side is never evaluated:

```
=IF(A1=0, 0, 1/A1)     with A1 = 0     →  0, not #DIV/0!
```

Every other function is strict; `IF` is the only short-circuiting form in v1.

Ranges are pushed as *range operands* and consumed by the aggregate functions, so
`SUM(A1:A1000)` never materialises a thousand intermediate values. A range used
where a scalar is expected falls back to **implicit intersection** (the formula's
row for a one-column range, its column for a one-row range, its own cell for a
rectangle containing it, otherwise `#VALUE!`), which is what spreadsheets do.

### Ranges are watched, never expanded

The interesting design decision. A formula reading a range is recorded as a single
*watch* `(range, formula)`; the graph holds edges only for direct cell references.

The tempting alternative — expanding `SUM(A1:A100000)` into a hundred thousand
edges — is quadratic in disguise, because editing any cell inside the range
rewrites that whole edge set, and `StableGraph::remove_edge` is `O(degree)`. It
took an 8.4-second edit on a 100k-row sheet to make that concrete (see
[Performance](#6-performance)). With watches, the same edit touches two cells.

The two questions the scheduler asks are answered directly from the watch list:

* *who reads this cell?* → every watch whose range contains it;
* *what must this formula wait for?* → the dirty cells that watch list points at.

### Incremental recalculation

Editing a cell runs a five-step pipeline (`Sheet::apply_input`, `Sheet::recalculate`):

1. **detach** — drop the edited cell's previous precedents (edges and watches);
2. **store** — write the new input; a formula starts out unevaluated;
3. **attach** — rebuild edges from the compiled program's precedents;
4. **mark** — take the transitive closure of dependents: the *dirty* set;
5. **schedule** — levelise the dirty set and evaluate each level.

Levelisation is Kahn's algorithm over the subgraph *induced by the dirty set*, so
a precedent that is already up to date imposes no ordering and no recomputation.
Cells that become ready together are mutually independent, which is exactly the
condition for evaluating them in parallel — `rayon` runs any level with more than
one cell, and the results are collected before being written back, so the sheet is
only ever read while a level runs.

`RecalcReport` returns the dirty set, the levels actually run, and any cycles, so
behaviour is assertable in tests (e.g. "editing one cell of a 100k-row sheet
recomputes exactly 2 cells").

### Cycles: detected, not survived

A circular reference is reported as `#CYCLE!` on the cells that are *on* the cycle.
The cells merely downstream of one are still evaluated — they read the cycle error
as an ordinary value.

Cycles are found with Tarjan's algorithm (iteratively: a long dependency chain must
not overflow the stack) over the unresolved dirty cells. The first implementation
instead peeled dependency "ends" — which misclassifies a downstream cell that
itself has dependents. A property test caught it: a cell reading a self-referencing
cell was marked part of the cycle, and the incremental result disagreed with a full
recalculation.

### Copy and fill

Filling is parse → shift → print → recompile (`Expr::shifted`). Relative components
move, `$` components do not, and a reference pushed off the sheet becomes `#REF!`
rather than silently pointing somewhere wrong. The AST's `Display` is a canonical
printer that emits the minimum parentheses needed to re-parse to the same tree,
which is what makes this safe; a property test checks the round trip for arbitrary
trees.

Literals are copied verbatim — there is no series detection, so copying `1, 2` does
not produce `3, 4`.

---

## 2. Formula language (v1)

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

### Parse errors are positional

Since tokens carry spans, the parser reports byte offsets into the original text,
and `Diagnostic::render` draws a caret:

```
=1 + 2)
      ^ unexpected `)`; expected end of formula
```

The app shows that rendering in the status bar for the cell being edited.

---

## 3. File format

A native JSON format via `serde` (`engine/src/io.rs`), versioned by `FORMAT_VERSION`.
It stores **inputs only** — literals and formula *text* — never computed values, so
a file stays small, diffable and correct if the evaluator ever changes. Loading
recompiles and recalculates everything. Newer versions are rejected rather than
misread, and out-of-range records are skipped rather than panicking.

```json
{
  "version": 1,
  "cells": [
    { "row": 1, "col": 2, "kind": "number",  "value": 12.0 },
    { "row": 4, "col": 2, "kind": "formula", "source": "=SUM(C2:C4)" }
  ]
}
```

`.xlsx` import/export via `calamine` / `rust_xlsxwriter` was left out deliberately:
it is the one part of the brief marked optional, and the native format was to be
done first.

---

## 4. UI (iced 0.14, Elm architecture)

* `app/src/application.rs` — state, messages, update loop, widget tree.
* `app/src/grid.rs` — the canvas: viewport maths, drawing, pointer reporting.
* `app/src/theme.rs` — the palette and chrome styles.

### Virtualisation

The canvas draws only the rows and columns that intersect the viewport (plus one
cell of buffer). Scrolling, drawing and hit-testing are pure functions of the
scroll offset and viewport size, so a frame costs what the *window* costs, not what
the sheet costs. There is no per-cell allocation while drawing, and the sheet is
only asked for values inside the visible range.

### Garden lattice

Warm whitewash and cream (`#FAF8F3`, `#F5F1E8`), soft sage structure lines
(`#D8DFD3`) that read as trellis rather than grid, and a small range of
low-contrast greens (sage `#9BAF93` → moss `#7D9471` → leaf `#5C7F52`) for the
active cell, fill handle, selection gutters and buttons. The selection is a
translucent green wash, not a hard box. Text is warm charcoal (`#3E3A34`), errors
are muted clay (`#9C5F4E`), and numbers are right-aligned while everything else is
left-aligned.

### Interaction

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
| `Ctrl`+`S` / `Ctrl`+`O` | save to / load from `lattice-sheet.json` |

While no editor is focused the keyboard drives the grid; typing starts an edit,
which focuses the in-cell editor and re-routes the keyboard to it. `Escape` is the
one key forwarded past a focused editor, so an edit can always be abandoned.

The status bar doubles as instrument panel: it shows the parse error for the active
cell, the sum and average of a selected range, or what the last recalculation did
("filled 7 cells (recalculated 7 in 2 levels)").

---

## 5. Testing

186 tests: 152 in the engine, 34 in the app.

* **Unit tests** per module, including the geometry (`Metrics`) and the app's
  update loop, which are pure functions and need no window.
* **Property tests** (`engine/tests/properties.rs`) for the invariants that
  examples cannot cover: parse/print round trips, shape-preserving shifts, parse
  safety on arbitrary input, postfix evaluation agreeing *bit for bit* with an
  independent tree evaluator, range aggregation matching explicit addition, and
  the engine's central promise — that recalculating again changes nothing.
* **Scale tests** (`engine/tests/scale.rs`) assert work done, not wall-clock: an
  edit among 100k rows recomputes exactly two cells; a 20k-cell chain is linear; a
  whole-column range adds no edges.

The property tests earned their keep by finding real bugs:

| Bug | Found by |
| --- | --- |
| `#DIV/0!` swallowed following `/` (`#DIV/0!/2` became one token) | parse/print round trip |
| `Graph::remove_node` swapping the last node in, invalidating stored indices | graph pruning test |
| Cells downstream of a cycle misreported as part of it | recalc-idempotence property |
| Quadratic range-edge materialisation (8.4 s per edit on 100k rows) | scale test |
| Quadratic leveliser (43 s for a 20k-cell chain) | scale test |
| Pointer hit-testing offset by the chrome height (`position_over` is window-absolute) | driving the GUI with synthetic X input |

The last one is worth noting: `iced`'s `Cursor::position_over` returns *window*
coordinates filtered by the widget's bounds, so the canvas origin has to be
subtracted explicitly. Unit tests could not see it; clicking a cell in a real
window could.

---

## 6. Performance

`cargo run --release -p engine --example recalc_bench`, 100 000 populated rows plus
a whole-column aggregate:

| Operation | Time |
| --- | --- |
| fill 100 000 cells | 45 ms |
| add `=SUM(A1:A100000)` | 1.9 ms |
| edit one cell in the middle | **1.2 ms** (2 cells recomputed) |
| full recalculation (100 001 cells) | 54 ms |
| serialise to JSON (9.2 MB) | 27 ms |

The edit is the number that matters: it is constant work, because ranges are
watches rather than edges and because only the dirty subgraph is touched.

---

## 7. Known limitations

* Single sheet per workbook; no cell formatting, borders, or column widths.
* No named ranges, absolute-sheet references, or cross-sheet formulas.
* No `AND`/`OR`/`NOT`, no short-circuiting beyond `IF`, no text functions beyond
  `CONCAT`.
* Text is never coerced to a number on entry (`"5"` stays text); numbers are never
  rendered as text.
* Series detection on fill (turning `1, 2` into `3, 4`) is deliberately absent.
* Column widths are fixed; a long text cell is truncated with an ellipsis and a
  long number is rounded to fit.
* `.xlsx` interoperability is not implemented (see §3).

### Where the next hour would go

1. `calamine` / `rust_xlsxwriter` round trip, to make the format claim complete.
2. Per-column widths, since the drawing code already computes cell rectangles in
   one place (`Metrics`).
3. Undo/redo: `Sheet` is `Clone` and edits already return the dirty set, so a
   snapshot-per-edit stack is a small step.
