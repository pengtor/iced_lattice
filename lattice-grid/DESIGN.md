# lattice-grid: the SheetModel seam

**Status:** design only. Nothing in this file is wired into the widget yet.

## Goal

`lattice-grid` must render and interact with a spreadsheet-shaped data source
without knowing what produced it. Today it names `engine` directly in four
places:

```
sheet.rs:7    use engine::{CellRef, Bounds, Sheet, Value, MAX_COLS, MAX_ROWS};
sheet.rs:329  let full = engine::format_number(value);
sheet.rs:646  let label = engine::addr::col_name(col);
Cargo.toml    engine = { path = "../engine" }
```

After this split, `engine` is one implementor of `SheetModel` and nothing more.

## The constraint that decides the hard question

A host cannot write `impl SheetModel for engine::Sheet` — both the trait and the
type are foreign to the host crate. The orphan rule leaves exactly three places
for that impl to live:

| impl lives in | consequence |
| --- | --- |
| the host crate, via a **local newtype** | legal, ~20 lines, and it is the host's own file |
| `lattice-grid` | the widget depends on `engine` — the thing we are removing |
| `engine` | engine depends on `lattice-grid`, which pulls in `iced` — layering inversion |

So the host adapter is a local newtype. That is true under every design below,
which is what makes the `CellRef` question separable from the trait question:
the trait shape is expensive to change later; where `CellRef` physically lives
is a type swap.

**Decision: the widget owns its cell vocabulary.** `lattice_grid::CellRef` and
`lattice_grid::Bounds`, not `engine`'s. Reasons, in order of weight:

1. The orphan rule means a host cannot adapt a foreign cell type to the widget
   in either direction without a mapping newtype. Owning the type is the only
   way the widget's public API is usable without friction.
2. It requires no changes to `engine` (232 tests, plus the README-extracted
   suite, plus serde derives, plus formula-only types like `Ref`/`RangeRef`
   sharing a module with `CellRef`).
3. For an external host the ergonomics are identical either way — they index
   `cell.row` / `cell.col`.
4. The cost is confined to our own example, which is the honest place to show
   the seam rather than hide it.

Rejected alternative: a shared `lattice-core` crate holding `CellRef`/`Bounds`/
`format_number`, re-exported by both `engine` and `lattice-grid`. It removes
~6 conversion sites *inside our example* and unifies the number formatter. It
costs a third publishable crate, a serde feature split (only `CellRef`/`Ref`
derive serde today, and `Ref`/`RangeRef` are formula concerns that must not
travel into the widget), and surgery on a stable module. Same host ergonomics,
same adapter requirement. Revisit only if a third consumer of that vocabulary
appears; the trait signature stays `value(&self, CellRef) -> CellValue` either
way, so the swap is cheap.

## The sketch

```rust
/// A cell address. (0, 0) is A1. The widget's own type: it never names a host's.
pub struct CellRef { pub row: u32, pub col: u32 }
impl CellRef { pub const fn new(row: u32, col: u32) -> Self }

/// A rectangular selection. Fields match engine::Bounds so a mapping is a copy.
pub struct Bounds { pub min_row: u32, pub max_row: u32, pub min_col: u32, pub max_col: u32 }
impl Bounds { pub fn new(a: CellRef, b: CellRef) -> Self; pub fn single(cell: CellRef) -> Self; pub fn contains(&self, cell: CellRef) -> bool }

/// How many cells the sheet has.
pub struct Dims { pub rows: u32, pub cols: u32 }
impl Dims { pub const SPREADSHEET: Dims = Dims { rows: 1_048_576, cols: 16_384 } }

/// Everything the widget needs in order to paint one cell.
pub enum CellValue {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
}

/// Read-only. Edits travel out through GridEvent, exactly as selection does.
pub trait SheetModel {
    fn dims(&self) -> Dims;
    fn value(&self, cell: CellRef) -> CellValue;

    /// The extent of actual data, for Ctrl+Arrow navigation. Defaulted to the
    /// whole sheet: a model with no used-range concept degrades to
    /// "jump to the sheet's edge" rather than to a panic or an empty range.
    /// A host that knows its data overrides it -- the example's adapter does.
    fn used_bounds(&self) -> Bounds {
        Bounds::new(CellRef::new(0, 0), self.dims().last_cell())
    }
}
```

## The controller layer

`GridEvent` and `Metrics` are the boundary and stay the boundary. They are also
a *lot* of boundary: a host that wants a working grid has to reimplement
hit-testing, the drag state machine, shift-extend, Ctrl+Arrow, thumb dragging,
track paging and double-click timing — roughly the first 300 lines of
`examples/spreadsheet/src/input.rs` as it stood before the split.

So `lattice-grid/src/controller.rs` ships those mechanics as `GridController`,
the way `iced_table` ships its divider and resize logic in the library. It is
**an optional convenience, not a replacement**: nothing in `GridEvent`,
`Metrics` or `GridProgram` changed to make room for it, and a host that wants
to do its own hit-testing still can.

The split is by *who can answer*, not by layer:

| the controller owns | the host owns |
| ------------------- | ------------- |
| selection, scroll, the drag in flight, the double-click clock | the sheet |
| hit-testing: fill handle, scrollbar, gutters, cells | opening the editor, writing a fill, clearing cells, recalculation |
| keyboard movement (`move_selection`), gutters, paging | key bindings, the name box, notices |

The host's half is expressed as `Hooks<M>` — four `fn` pointers returning the
host's own message type, so the hooks land in the host's `update` where its data
is reachable. That is what keeps formula semantics out of the widget: a
spreadsheet shifts relative references on a fill, a chart host might repeat a
literal, and the controller does not need to know which.

Two API decisions worth recording:

* `GridController` is **not** generic over the model. A host borrows its data
  into a `SheetModel` adapter for the duration of a frame, so a controller that
  owned the adapter would be self-referential (`GridController<SheetView<'a>>`
  cannot be a field of a struct that owns the sheet the view borrows). The
  model is a parameter instead, which also keeps the borrow of the host's sheet
  disjoint from the borrow of its controller.
* Clearing cannot be mechanical. A generic controller may not scan
  `Dims::SPREADSHEET` for populated cells — 17 billion `value()` calls — and
  the widget is read-only, so `request_clear` reports the bounds and the host
  clears. Same reason `on_fill_committed` reports two blocks instead of
  filling: the widget never writes.

## Question 1: does the widget need write access?

**No — read-only.** Evidence, not preference:

* `GridProgram::draw` calls `self.sheet.value(cell)` and nothing else. The only
  method on `engine::Sheet` the widget ever touches is `value`.
* In-cell editing and the formula bar are iced widgets in
  `examples/spreadsheet/src/application.rs`, not canvas code. The widget never
  sees them.
* Fill-handle dragging is *preview only* inside the widget
  (`fill_preview: Option<Bounds>` is painted as an overlay); `input.rs` applies
  the fill after the pointer is released.
* `GridEvent` already establishes the pattern: the widget reports, the host
  mutates.

A `set(&mut self, ...)` would force the widget to hold `&mut`, which iced's
`canvas::Program` (all `&self`) cannot do without interior mutability, and it
would make the widget a party to validation, undo, and recalc. Reporting is the
right shape and it already exists.

## Question 2: what does `CellValue` carry?

Only what painting needs. The draw loop touches a value in exactly four ways:

```rust
if value.is_empty() { continue }                       // skip
let align = horizontal_alignment(&value);              // Number -> Right, else Left
match &value { Value::Number(n) => fit_number(*n, ...), other => fit_text(&other.as_text(), ...) }
let color = if value.is_error() { clay } else { ink };
```

So the variants are the display vocabulary and nothing more. Explicitly **not**
included: formula source, dependency edges, recalc state, number formats,
`ErrorKind` (a `String` of already-rendered text is enough for the cell and for
the colour test).

Why an enum rather than `{ text: String, align, error: bool }`: `fit_number`
re-rounds when a number does not fit, which requires the `f64`. A pre-formatted
string cannot be shortened correctly.

`Bool` is kept even though it currently paints exactly like `Text` (engine
renders `TRUE`/`FALSE`, left-aligned). It costs one variant and makes the
adapter a 5-to-5 mapping that is easy to verify.

**Alignment is derived from the variant**, matching engine's behaviour today
(only `Number` is right-aligned — `value.rs`/`sheet.rs` test
`numbers_are_right_aligned_and_text_is_left_aligned` asserts precisely this).
Known limitation, recorded deliberately: a host with right-aligned text (a
currency or date formatted string) has nowhere to put that hint. The fix, if it
is ever needed, is `Text { text, align }`; it is deferred because inventing it
now would change rendering, and this phase must not.

## Question 3: fixed limits or dynamic extent?

**`Dims` on the trait, and the method is required — no default.**

* Fixed widget constants would make a 100×100 host sheet permanently carry a
  dead scrollbar: `Scrolling::thumb` only returns `None` when content fits the
  track, so a smaller `Dims` is exactly what makes scrollbars disappear.
* A defaulted `fn dims(&self) -> Dims { Dims::SPREADSHEET }` is a lie for most
  hosts, and the failure mode ("my scrollbar goes nowhere") is confusing. One
  required line, spelled `Dims::SPREADSHEET` when that is genuinely what the
  host is, keeps the contract honest.
* The numbers stay available as a named constant because 1,048,576 × 16,384 is
  the Excel limit — a spreadsheet convention, not an engine internal. `engine`
  keeps its own `MAX_ROWS`/`MAX_COLS`; the widget stops importing them.

## Where `format_number` and `col_name` land

Checked both, as asked. Neither needs anything engine-specific: they read a
`f64` / a `u32` and return a `String`, and touch no engine state.

* `engine::format_number` — **presentation, moves to the widget.** Engine uses
  it for `Value::as_text` (display), the lexer's error messages, and it is
  public API. Note `format_number_exact` (used by `ast.rs` for formula
  round-trip) is *not* presentation and stays in engine.
* `engine::addr::col_name` — **presentation, moves to the widget.** Engine uses
  it for `CellRef::a1`/`Display`/parsing; the widget needs it only for the
  column gutter.

This means the widget exports `format_number`, and engine keeps its own copy —
duplication forced by layering (engine cannot depend on an iced crate). It is
acceptable here for two reasons: the widget's copy is needed anyway for
`fit_number`, and the drift risk is testable rather than theoretical. The
example depends on both crates, so **one test in the example asserts
`lattice_grid::format_number(x) == engine::format_number(x)` over engine's own
vectors** (`0.1 + 0.2 -> "0.3"`, `1.0/3.0 -> "0.333333333333333"`, `1e21`,
`2.5e-10`, `1.5e300`, `-0.0 -> "0"`, `1234567890123456`). If that test ever
fails, the fix is the shared-crate option above, which is a mechanical move.

`col_name` needs no such guard: column letters are an external convention with
a fixed answer (26 variants, `XFD` at 16383). Duplicating a fixed convention is
harmless; duplicating a rounding *policy* is not.

## `GridProgram` and `Metrics` after the change

```rust
pub struct GridProgram<'a, M> {
    pub model: &'a dyn SheetModel,   // was: sheet: &'a Sheet
    pub selection: Bounds,
    pub active: CellRef,
    pub fill_preview: Option<Bounds>,
    pub scroll: Vector,
    pub active_scrollbar: Option<Axis>,
    pub palette: GardenPalette,
    pub on_event: fn(GridEvent) -> M,
}

pub struct Metrics { pub scroll: Vector, pub viewport: Size, pub dims: Dims }
impl Metrics { pub fn new(scroll: Vector, viewport: Size, dims: Dims) -> Self }
```

**Superseded during implementation.** The field is `pub model: S` where
`S: SheetModel` — owned, so `GridProgram<M, S>` has no lifetime parameter at
all. A `&'a dyn SheetModel` was the sketch's choice (object-safe, ~360 calls
per frame, no `Send` bound in `iced_widget-0.14.2/src/canvas/program.rs`), but
it cannot work: the element returned by `Lattice::view` would borrow an adapter
that dies at the end of that function. Owning the model moves it into the
`Canvas` instead, which the host can build inline — and it buys static dispatch
for free.

`Metrics::new` gains a third argument; every `MAX_ROWS`/`MAX_COLS` inside it
becomes `self.dims.rows`/`self.dims.cols`. ~12 call sites in the example and
all 20 widget tests update mechanically.

The return type that forced the change above: `view(&self) -> Element<'_, Message>`
means anything the element borrows must outlive `&self`, and a local adapter
cannot. Owning the model sidesteps it — the host names the adapter where it
builds the widget:

```rust
let grid = canvas(GridProgram { model: SheetView(&self.sheet), .. });
```

## The host adapter (the example's one new file)

`examples/spreadsheet/src/model.rs`, ~30 lines, the only place the two crates
meet. It replaces `sheet: &self.sheet` with `model: &view` and provides the
`Dims` the example's `Metrics` needs:

```rust
pub struct SheetView<'a>(pub &'a Sheet);
pub fn dims() -> Dims { Dims { rows: engine::MAX_ROWS, cols: engine::MAX_COLS } }

impl SheetModel for SheetView<'_> {
    fn dims(&self) -> Dims { dims() }
    fn value(&self, cell: CellRef) -> CellValue {
        match self.0.value(engine::CellRef::new(cell.row, cell.col)) {
            Value::Empty => CellValue::Empty,
            Value::Number(n) => CellValue::Number(n),
            Value::Text(t) => CellValue::Text(t),
            Value::Bool(b) => CellValue::Bool(b),
            Value::Error(k) => CellValue::Error(k.to_string()),
        }
    }
}
```

Neither `impl From<engine::CellRef> for lattice_grid::CellRef` nor the reverse
can live in the example (both types foreign). The mapping is field access, so
two one-line helpers in `model.rs` (`grid_cell`, `sheet_cell`, `grid_bounds`)
carry the ~15 widget-boundary call sites in `input.rs` and `application.rs`.

## Proof the abstraction works

Not "it compiles against engine's impl" — a test double in the widget's own
test module, with `engine` gone from `lattice-grid/Cargo.toml` entirely
(including dev-dependencies):

```rust
struct Fixture { /* not engine, not Sheet */ }
impl SheetModel for Fixture { .. }
```

Three properties, all checkable without a renderer:

1. A small `Dims` (say 5 × 3) makes `visible_rows` clamp to `0..5`, `cell_at`
   return `None` past it, `clamp_scroll` stop at its extent, and both
   scrollbars vanish — the dynamic-extent claim, proven.
2. `CellValue::Number` vs `Text` vs `Error` select right-alignment, left-
   alignment, and the error colour — the paint decision, proven through the
   pure helpers (`CellValue::alignment()`, `is_empty()`, `is_error()`) that the
   existing `numbers_are_right_aligned_and_text_is_left_aligned` test will then
   exercise without naming `engine::Value`.
3. `format_number`/`col_name` carry their own vectors (engine's, copied), so the
   widget's display rules are pinned independently of engine.

## Planned changes (step 2 — landed)

1. `lattice-grid/src/sheet.rs`: add `CellRef`/`Bounds`/`Dims`/`CellValue`/
   `SheetModel` (or a `model.rs` beside it); `model: &'a dyn SheetModel`;
   `Metrics` gains `dims`; `format_number`/`col_name` move in; the `use engine::`
   line and the `engine::` call sites at 329/646 go.
2. `lattice-grid/Cargo.toml`: drop the `engine` dependency.
3. `lattice-grid/src/lib.rs`: export the new surface.
4. `examples/spreadsheet/src/model.rs`: new adapter.
5. `examples/spreadsheet/src/{application,state,input}.rs`: `model: &view`,
   `Metrics::new(.., dims)`, `grid_cell`/`sheet_cell`/`grid_bounds` at the
   boundary.
6. Re-run all 11 suites; expect 359 kept green plus the new double/drift tests.
