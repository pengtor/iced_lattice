# lattice-grid

A virtualised spreadsheet grid for [iced](https://iced.rs). Only the visible
cells are drawn, so scrolling a 1,000,000-row sheet costs the same per frame as
scrolling a ten-row one.

The grid is message-agnostic and read-only. Your data goes in through one trait,
and everything the pointer does comes back out as a `GridEvent` for your
application to interpret. It depends on `iced_core` and `iced_widget` only — no
windowing, no application runtime — so it drops into an iced application you
already have.

## 1. Implement `SheetModel`

Two methods, both read-only:

```rust
use lattice_grid::{CellRef, CellValue, Dims, SheetModel};

#[derive(Clone, Copy)]
struct Scores;

impl SheetModel for Scores {
    /// How big the sheet is. The grid never asks for a cell outside this.
    fn dims(&self) -> Dims {
        Dims { rows: 5, cols: 5 }
    }

    /// What a cell holds. Called once per visible cell per frame, so answer for
    /// that cell only — don't rebuild the sheet.
    fn value(&self, cell: CellRef) -> CellValue {
        match (cell.col, cell.row) {
            (0, 0) => CellValue::Text("player".into()),
            (1, 0) => CellValue::Text("score".into()),
            (0, 1) => CellValue::Text("ada".into()),
            (1, 1) => CellValue::Number(41.5),
            (1, 2) => CellValue::Number(-3.0),
            _ => CellValue::Empty,
        }
    }
}
```

`CellRef { row, col }` is zero-based, so `CellRef::new(0, 0)` is `A1`.

The grid holds the model by value, so keep the type cheap to hand over — a unit
struct here, a `Copy` view over your real data in the reference implementation.

`Dims` is the sheet's extent; `Dims::SPREADSHEET` is the familiar
1,048,576 × 16,384.

`CellValue` carries what to paint, and nothing else:

| variant | painted as |
| ------- | ---------- |
| `CellValue::Empty` | nothing |
| `CellValue::Number(f64)` | right-aligned, rounded to fit the column |
| `CellValue::Text(String)` | left-aligned, truncated to fit |
| `CellValue::Bool(bool)` | `TRUE` or `FALSE`, left-aligned |
| `CellValue::Error(String)` | the message, left-aligned |

Numbers are rendered with `lattice_grid::format_number`, which is public so a
host's own formatting can be made to agree with the grid's.

## 2. Wire it into a view

```rust
use iced::widget::canvas;
use iced::{Element, Length, Vector};
use lattice_grid::{Bounds, CellRef, GardenPalette, GridEvent, GridProgram};

#[derive(Debug, Clone)]
enum Message {
    /// The grid's events, forwarded into your own message type.
    Grid(GridEvent),
}

struct App {
    model: Scores,
    selection: Bounds,
    active: CellRef,
    scroll: Vector,
}

impl App {
    fn view(&self) -> Element<'_, Message> {
        canvas(GridProgram {
            // Cheap to hand over: `Scores` is `Copy`, and the reference
            // implementation passes a `Copy` view for the same reason.
            model: self.model,
            selection: self.selection,
            active: self.active,
            fill_preview: None,      // Some(bounds) while a fill drag is in flight
            scroll: self.scroll,
            active_scrollbar: None,  // Some(axis) while a bar is held
            palette: GardenPalette::light(),
            on_event: Message::Grid, // fn(GridEvent) -> Message
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}
```

Start from `Bounds::single(CellRef::new(0, 0))`, `active: CellRef::new(0, 0)` and
`scroll: Vector::new(0.0, 0.0)`. The grid keeps no state of its own: selection,
scroll offset and in-flight drags all belong to the host, so re-rendering with
the same fields paints the same thing.

## 3. Answer the events

`GridEvent` is the entire boundary. Nothing is ever written through
`SheetModel`.

| event | what you do with it |
| ----- | ------------------- |
| `PointerPressed { position, viewport }` | hit-test, set `selection`/`active`, start a drag |
| `PointerMoved { position, viewport }` | extend a selection, or drag a fill handle or thumb |
| `PointerReleased` | commit the drag; open your editor if this was a double-click |
| `Scrolled { delta, viewport }` | `scroll = metrics.clamp_scroll(scroll + delta)` |
| `Viewport(size)` | remember the canvas size — the hit-testing needs it |

Do the geometry with `Metrics`, which is the same arithmetic the grid paints
with:

```rust
// Inside your `PointerPressed` handler.
use lattice_grid::{Axis, Metrics, ScrollbarHit};

let metrics = Metrics::new(self.scroll, viewport, self.model.dims());

// A press landed somewhere. Which part of the grid was it?
match metrics.scrollbar_at(position) {
    Some(ScrollbarHit::Thumb(axis)) => { /* start dragging that thumb */ }
    Some(ScrollbarHit::Track { axis, forward }) => {
        let page = metrics.page_scroll(axis, forward);
        self.scroll = metrics.clamp_scroll(axis.with(self.scroll, axis.of(self.scroll) + page));
    }
    None => {
        // Headers, the fill handle, or a cell.
        if metrics.hits_fill_handle(self.selection, position) {
            // start a fill drag
        } else if let Some(cell) = metrics.cell_at(position) {
            // select it
        }
    }
}
```

The rest of the useful calls:

| call | gives you |
| ---- | --------- |
| `metrics.cell_at(position)` | the `CellRef` under a point, or `None` on a header or off the sheet |
| `metrics.cell_rect(cell)` | where one cell is on screen — put your text field here |
| `metrics.bounds_rect(bounds)` | where a whole selection is on screen |
| `metrics.thumb_drag(axis, pixels)` | dragged pixels, as a scroll-offset delta for that axis |
| `metrics.scroll_to_show(cell, scroll)` | the offset that brings a cell into view |
| `metrics.clamp_scroll(scroll)` | an offset pulled back inside the sheet's limits |

Editing is the host's business: open your own text field over
`metrics.cell_rect(self.active)`, and write the result into your own data. The
trait has no write method on purpose — the grid cannot invent an edit.

## Reference implementation

[`examples/spreadsheet/src/model.rs`](../examples/spreadsheet/src/model.rs) is
the fuller version: it adapts a real recalculation engine to `SheetModel`,
including the newtype that maps one crate's cell vocabulary onto the other's and
a test pinning the widget's number formatting to the engine's. A host with its
own data source writes exactly that file, and nothing else.

## Styling

`GardenPalette` holds the grid's colours as plain `Color` fields.
`GardenPalette::light()` and `GardenPalette::dark()` are the two defaults; build
your own and pass it as `palette`. `HAIRLINE` and `FOCUS_BORDER` are exported
for matching your own chrome to the grid's.
