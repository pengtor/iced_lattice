//! An optional interaction layer for hosts that don't want to reimplement
//! grid mechanics.
//!
//! [`GridEvent`] and [`Metrics`] stay the low-level primitives — nothing here
//! replaces them, and a host that wants full control can keep using them
//! directly. A [`GridController`] is a convenience on top: it owns the
//! interaction state (selection, scroll, drag, double-click timing), does the
//! hit-testing, and calls back into the host for the decisions only the host
//! can make — opening an editor, writing a fill, clearing cells.
//!
//! The controller never touches the model. It reads it through
//! [`SheetModel`] and reports what it wants done through [`Hooks`], which is
//! what keeps a spreadsheet's formula semantics out of the widget and lets a
//! non-spreadsheet host reuse the same mechanics.
//!
//! ```
//! use lattice_grid::{GridController, Selection, CellRef};
//!
//! let mut grid = GridController::new();
//! assert_eq!(grid.selection, Selection::single(CellRef::new(0, 0)));
//! ```

use std::time::{Duration, Instant};

use iced_core::{Point, Size, Vector};

use crate::model::{Bounds, CellRef, SheetModel};
use crate::sheet::{
    Axis, GridEvent, Metrics, ScrollbarHit, CELL_HEIGHT, CELL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH,
};

/// How long a second click on the same cell still counts as a double click.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// The selection, as an anchor plus a cursor.
///
/// Stored as two corners rather than a [`Bounds`] because `Bounds` is
/// normalised: it would forget which corner the user started from, and that
/// is exactly what shift-extend and fill need.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub anchor: CellRef,
    pub active: CellRef,
}

impl Selection {
    pub const fn single(cell: CellRef) -> Selection {
        Selection { anchor: cell, active: cell }
    }

    pub fn bounds(&self) -> Bounds {
        Bounds::new(self.anchor, self.active)
    }

    pub fn is_single(&self) -> bool {
        self.anchor == self.active
    }

    pub fn contains(&self, cell: CellRef) -> bool {
        self.bounds().contains(cell)
    }
}

/// What the pointer is currently doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Selecting,
    /// A fill in progress; the bounds are the target as it grows.
    Filling(Bounds),
    /// `last` turns pointer movement into a scroll delta.
    Scrollbar { axis: Axis, last: Point },
}

/// The decisions only the host can make, as `on_event` is for [`GridProgram`].
///
/// Each one is answered with the host's own message type, so a controller
/// call returns messages the host can hand straight to `Task::batch`.
///
/// [`GridProgram`]: crate::GridProgram
pub struct Hooks<M> {
    /// A gesture asked for an editor on this cell (a double click). Typing
    /// your own keys is host business and doesn't come through here.
    pub on_edit_requested: fn(CellRef) -> M,
    /// A fill drag finished. Only the host knows how to fill: a spreadsheet
    /// shifts the source's relative references, another host might repeat a
    /// literal.
    pub on_fill_committed: fn(Bounds, Bounds) -> M,
    /// A drag-select finished on more than one cell.
    pub on_selection_settled: fn(Bounds) -> M,
    /// Clear was asked for on this range. The widget is read-only, so the
    /// host does the clearing.
    pub on_clear_requested: fn(Bounds) -> M,
}

/// Selection, scroll and drag state, plus the mechanics that maintain them.
///
/// Not generic over the model on purpose: a host usually borrows its data
/// into a [`SheetModel`] adapter for the duration of a frame, so the adapter
/// can't be owned here. The model is passed to each call instead, which also
/// keeps the borrow of the host's sheet disjoint from the borrow of its
/// controller.
#[derive(Clone, Copy, Debug)]
pub struct GridController {
    pub selection: Selection,
    pub scroll: Vector,
    pub drag: Option<Drag>,
    last_click: Option<(Instant, CellRef)>,
}

impl Default for GridController {
    fn default() -> GridController {
        GridController::new()
    }
}

impl GridController {
    pub fn new() -> GridController {
        GridController {
            selection: Selection::single(CellRef::new(0, 0)),
            scroll: Vector::new(0.0, 0.0),
            drag: None,
            last_click: None,
        }
    }

    fn metrics<S: SheetModel>(&self, model: &S, viewport: Size) -> Metrics {
        Metrics::new(self.scroll, viewport, model.dims())
    }

    fn scroll_to<S: SheetModel>(&mut self, model: &S, viewport: Size, cell: CellRef) {
        let metrics = self.metrics(model, viewport);
        self.scroll = metrics.scroll_to_show(cell, self.scroll);
    }

    /// Keyboard-driven movement. `extend` keeps the anchor (shift) and `jump`
    /// goes to the edge of the data (ctrl) instead of one step.
    pub fn move_selection<S: SheetModel>(
        &mut self,
        model: &S,
        viewport: Size,
        row_delta: i32,
        col_delta: i32,
        extend: bool,
        jump: bool,
    ) {
        let dims = model.dims();
        let active = self.selection.active;
        let target = if jump {
            self.jump_target(model, row_delta, col_delta)
        } else {
            CellRef::new(
                (active.row as i64 + row_delta as i64).clamp(0, dims.rows as i64 - 1) as u32,
                (active.col as i64 + col_delta as i64).clamp(0, dims.cols as i64 - 1) as u32,
            )
        };
        if extend {
            self.selection.active = target;
        } else {
            self.selection = Selection::single(target);
        }
        self.scroll_to(model, viewport, target);
    }

    /// The edge of the used range in the asked-for direction, never moving
    /// backwards past the active cell.
    fn jump_target<S: SheetModel>(&self, model: &S, row_delta: i32, col_delta: i32) -> CellRef {
        let active = self.selection.active;
        let used = model.used_bounds();
        CellRef::new(
            if row_delta > 0 {
                used.max_row.max(active.row)
            } else if row_delta < 0 {
                used.min_row.min(active.row)
            } else {
                active.row
            },
            if col_delta > 0 {
                used.max_col.max(active.col)
            } else if col_delta < 0 {
                used.min_col.min(active.col)
            } else {
                active.col
            },
        )
    }

    pub fn select_column<S: SheetModel>(&mut self, model: &S, col: u32) {
        let rows = model.dims().rows;
        self.selection = Selection {
            anchor: CellRef::new(0, col),
            active: CellRef::new(rows - 1, col),
        };
    }

    pub fn select_row<S: SheetModel>(&mut self, model: &S, row: u32) {
        let cols = model.dims().cols;
        self.selection = Selection {
            anchor: CellRef::new(row, 0),
            active: CellRef::new(row, cols - 1),
        };
    }

    /// Route a pointer or scroll event through the mechanics.
    ///
    /// `extend` is the shift state, which no [`GridEvent`] carries, and `now`
    /// is the clock for double-click detection — passed in so a test can
    /// drive it.
    pub fn handle<S: SheetModel, M>(
        &mut self,
        model: &S,
        event: &GridEvent,
        extend: bool,
        now: Instant,
        hooks: &Hooks<M>,
    ) -> Vec<M> {
        match event {
            GridEvent::PointerPressed { position, viewport } => {
                self.pointer_pressed(model, *position, *viewport, extend, now, hooks)
            }
            GridEvent::PointerMoved { position, viewport } => {
                self.pointer_moved(model, *position, *viewport);
                Vec::new()
            }
            GridEvent::PointerReleased => self.pointer_released(hooks),
            GridEvent::Scrolled { delta, viewport } => {
                let metrics = self.metrics(model, *viewport);
                self.scroll = metrics.clamp_scroll(self.scroll + *delta);
                Vec::new()
            }
            // The host keeps the viewport; nothing here depends on it.
            GridEvent::Viewport(_) => Vec::new(),
        }
    }

    pub fn pointer_pressed<S: SheetModel, M>(
        &mut self,
        model: &S,
        position: Point,
        viewport: Size,
        extend: bool,
        now: Instant,
        hooks: &Hooks<M>,
    ) -> Vec<M> {
        let dims = model.dims();
        let metrics = Metrics::new(self.scroll, viewport, dims);

        if metrics.hits_fill_handle(self.selection.bounds(), position) {
            self.drag = Some(Drag::Filling(self.selection.bounds()));
            return Vec::new();
        }

        // Checked before cells: the scrollbar covers the last column
        if let Some(hit) = metrics.scrollbar_at(position) {
            match hit {
                ScrollbarHit::Thumb(axis) => {
                    self.drag = Some(Drag::Scrollbar { axis, last: position });
                }
                ScrollbarHit::Track { axis, forward } => {
                    self.drag = None;
                    let page = metrics.page_scroll(axis, forward);
                    self.scroll =
                        metrics.clamp_scroll(axis.with(self.scroll, axis.of(self.scroll) + page));
                }
            }
            return Vec::new();
        }

        if position.y < HEADER_HEIGHT && position.x >= HEADER_WIDTH {
            let x = position.x - HEADER_WIDTH + self.scroll.x;
            let col = (x / CELL_WIDTH).floor();
            if col >= 0.0 && col < dims.cols as f32 {
                self.select_column(model, col as u32);
            }
            return Vec::new();
        }
        if position.x < HEADER_WIDTH && position.y >= HEADER_HEIGHT {
            let y = position.y - HEADER_HEIGHT + self.scroll.y;
            let row = (y / CELL_HEIGHT).floor();
            if row >= 0.0 && row < dims.rows as f32 {
                self.select_row(model, row as u32);
            }
            return Vec::new();
        }

        let Some(cell) = metrics.cell_at(position) else {
            return Vec::new();
        };

        let double_click = self
            .last_click
            .is_some_and(|(when, last)| last == cell && now.duration_since(when) <= DOUBLE_CLICK);
        self.last_click = Some((now, cell));

        if double_click {
            self.selection = Selection::single(cell);
            self.drag = None;
            return vec![(hooks.on_edit_requested)(cell)];
        }

        if extend {
            self.selection.active = cell;
        } else {
            self.selection = Selection::single(cell);
        }
        self.drag = Some(Drag::Selecting);
        Vec::new()
    }

    pub fn pointer_moved<S: SheetModel>(&mut self, model: &S, position: Point, viewport: Size) {
        let metrics = self.metrics(model, viewport);

        // A bar drag works off-sheet, so it skips cells
        if let Some(Drag::Scrollbar { axis, last }) = self.drag {
            self.drag = Some(Drag::Scrollbar { axis, last: position });
            let dragged = metrics.thumb_drag(axis, axis.along(position) - axis.along(last));
            self.scroll = metrics.clamp_scroll(axis.with(self.scroll, axis.of(self.scroll) + dragged));
            return;
        }

        let Some(cell) = metrics.cell_at(position) else {
            return;
        };
        match self.drag {
            Some(Drag::Selecting) => self.selection.active = cell,
            // Preview grows the source block towards the pointer cell
            Some(Drag::Filling(source)) => {
                self.drag = Some(Drag::Filling(padded_fill_target(source, Bounds::single(cell))));
            }
            Some(Drag::Scrollbar { .. }) | None => {}
        }
    }

    pub fn pointer_released<M>(&mut self, hooks: &Hooks<M>) -> Vec<M> {
        match self.drag.take() {
            Some(Drag::Filling(target)) => {
                vec![(hooks.on_fill_committed)(self.selection.bounds(), target)]
            }
            Some(Drag::Selecting) if !self.selection.is_single() => {
                vec![(hooks.on_selection_settled)(self.selection.bounds())]
            }
            Some(Drag::Selecting) | Some(Drag::Scrollbar { .. }) | None => Vec::new(),
        }
    }

    /// Ask the host to clear the current selection.
    pub fn request_clear<M>(&self, hooks: &Hooks<M>) -> Vec<M> {
        vec![(hooks.on_clear_requested)(self.selection.bounds())]
    }
}

/// The union of two blocks, i.e. a fill dragged from `source` towards
/// `target`. Pure geometry: what gets *written* is the host's business.
pub fn padded_fill_target(source: Bounds, target: Bounds) -> Bounds {
    Bounds::new(
        CellRef::new(source.min_row.min(target.min_row), source.min_col.min(target.min_col)),
        CellRef::new(source.max_row.max(target.max_row), source.max_col.max(target.max_col)),
    )
}

/// The bottom-right corner of a block.
pub fn last_cell(bounds: Bounds) -> CellRef {
    CellRef::new(bounds.max_row, bounds.max_col)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CellValue, Dims};

    /// A model with no engine behind it at all: six columns, sixty rows, and
    /// data only in B3:D6. This is the proof the mechanics don't need a
    /// spreadsheet.
    struct Fixture;

    impl SheetModel for Fixture {
        fn dims(&self) -> Dims {
            Dims { rows: 60, cols: 40 }
        }

        fn value(&self, _cell: CellRef) -> CellValue {
            CellValue::Empty
        }

        fn used_bounds(&self) -> Bounds {
            Bounds::new(CellRef::new(2, 1), CellRef::new(5, 3))
        }
    }

    #[derive(Debug, PartialEq)]
    enum Asked {
        Edit(CellRef),
        Fill(Bounds, Bounds),
        Settled(Bounds),
        Clear(Bounds),
    }

    fn hooks() -> Hooks<Asked> {
        Hooks {
            on_edit_requested: Asked::Edit,
            on_fill_committed: Asked::Fill,
            on_selection_settled: Asked::Settled,
            on_clear_requested: Asked::Clear,
        }
    }

    fn viewport() -> Size {
        Size::new(900.0, 500.0)
    }

    fn metrics(grid: &GridController) -> Metrics {
        Metrics::new(grid.scroll, viewport(), Fixture.dims())
    }

    fn press(grid: &mut GridController, position: Point) -> Vec<Asked> {
        grid.pointer_pressed(&Fixture, position, viewport(), false, Instant::now(), &hooks())
    }

    fn inside(rect: iced_core::Rectangle) -> Point {
        Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
    }

    #[test]
    fn clicking_selects_and_dragging_extends() {
        let mut grid = GridController::new();
        let m = metrics(&grid);
        let from = inside(m.cell_rect(CellRef::new(2, 1)));
        let to = inside(m.cell_rect(CellRef::new(5, 3)));

        press(&mut grid, from);
        assert_eq!(grid.selection, Selection::single(CellRef::new(2, 1)));

        grid.pointer_moved(&Fixture, to, viewport());
        assert_eq!(grid.selection.bounds(), Bounds::new(CellRef::new(2, 1), CellRef::new(5, 3)));

        let asked = grid.pointer_released(&hooks());
        assert_eq!(asked, vec![Asked::Settled(Bounds::new(CellRef::new(2, 1), CellRef::new(5, 3)))]);
        assert_eq!(grid.drag, None, "releasing ends the drag");
    }

    #[test]
    fn shift_clicking_extends_instead_of_moving_the_anchor() {
        let mut grid = GridController::new();
        let m = metrics(&grid);
        let first = inside(m.cell_rect(CellRef::new(2, 1)));
        let second = inside(m.cell_rect(CellRef::new(4, 2)));
        press(&mut grid, first);
        grid.pointer_pressed(
            &Fixture,
            second,
            viewport(),
            true,
            Instant::now(),
            &hooks(),
        );
        assert_eq!(grid.selection.anchor, CellRef::new(2, 1), "the anchor stayed put");
        assert_eq!(grid.selection.active, CellRef::new(4, 2));
    }

    #[test]
    fn clicking_a_gutter_selects_the_whole_row_or_column() {
        let mut grid = GridController::new();

        press(&mut grid, Point::new(10.0, HEADER_HEIGHT + 2.0));
        assert_eq!(grid.selection.bounds(), Bounds::new(CellRef::new(0, 0), CellRef::new(0, 39)));

        press(&mut grid, Point::new(HEADER_WIDTH + 2.0, 4.0));
        assert_eq!(grid.selection.bounds(), Bounds::new(CellRef::new(0, 0), CellRef::new(59, 0)));
    }

    #[test]
    fn arrow_keys_move_the_active_cell_and_shift_extends() {
        let mut grid = GridController::new();
        grid.move_selection(&Fixture, viewport(), 1, 0, false, false);
        grid.move_selection(&Fixture, viewport(), 0, 2, false, false);
        assert_eq!(grid.selection, Selection::single(CellRef::new(1, 2)));

        grid.move_selection(&Fixture, viewport(), 1, 1, true, false);
        assert_eq!(grid.selection.anchor, CellRef::new(1, 2));
        assert_eq!(grid.selection.active, CellRef::new(2, 3));
    }

    #[test]
    fn the_active_cell_cannot_leave_the_sheet() {
        let mut grid = GridController::new();
        grid.move_selection(&Fixture, viewport(), -1, -1, false, false);
        assert_eq!(grid.selection, Selection::single(CellRef::new(0, 0)));

        grid.move_selection(&Fixture, viewport(), i32::MAX, i32::MAX, false, false);
        assert_eq!(grid.selection, Selection::single(CellRef::new(59, 39)));
    }

    #[test]
    fn ctrl_arrow_jumps_to_the_edge_of_the_used_range() {
        let mut grid = GridController::new();
        assert_eq!(Fixture.used_bounds().max_row, 5, "the fixture's data ends here");

        // From A1, right and down land on the last used row and column.
        grid.move_selection(&Fixture, viewport(), 0, 1, false, true);
        assert_eq!(grid.selection.active, CellRef::new(0, 3));
        grid.move_selection(&Fixture, viewport(), 1, 0, false, true);
        assert_eq!(grid.selection.active, CellRef::new(5, 3));

        // And back the other way, to the first used row and column.
        grid.move_selection(&Fixture, viewport(), -1, 0, false, true);
        assert_eq!(grid.selection.active, CellRef::new(2, 3));
        grid.move_selection(&Fixture, viewport(), 0, -1, false, true);
        assert_eq!(grid.selection.active, CellRef::new(2, 1));
    }

    #[test]
    fn a_double_click_asks_the_host_for_an_editor() {
        let mut grid = GridController::new();
        let cell = CellRef::new(3, 2);
        let at = inside(metrics(&grid).cell_rect(cell));

        let now = Instant::now();
        assert!(grid.pointer_pressed(&Fixture, at, viewport(), false, now, &hooks()).is_empty());
        let asked = grid.pointer_pressed(&Fixture, at, viewport(), false, now, &hooks());
        assert_eq!(asked, vec![Asked::Edit(cell)]);
    }

    #[test]
    fn a_fill_drag_previews_then_hands_the_host_the_two_blocks() {
        let mut grid = GridController::new();
        let source = Bounds::single(CellRef::new(2, 1));
        let m = metrics(&grid);
        let anchor = inside(m.cell_rect(CellRef::new(2, 1)));
        press(&mut grid, anchor);
        grid.pointer_released(&hooks());

        // Grab the fill handle and drag to row 5.
        let handle = inside(m.fill_handle(source));
        press(&mut grid, handle);
        assert_eq!(grid.drag, Some(Drag::Filling(source)));

        let m = metrics(&grid);
        let dragged_to = inside(m.cell_rect(CellRef::new(5, 1)));
        grid.pointer_moved(&Fixture, dragged_to, viewport());
        assert_eq!(grid.drag, Some(Drag::Filling(Bounds::new(CellRef::new(2, 1), CellRef::new(5, 1)))));

        // The host is told both blocks and does the writing itself.
        assert_eq!(
            grid.pointer_released(&hooks()),
            vec![Asked::Fill(source, Bounds::new(CellRef::new(2, 1), CellRef::new(5, 1)))]
        );
    }

    #[test]
    fn dragging_the_scrollbar_thumb_scrolls_by_the_dragged_amount() {
        let mut grid = GridController::new();
        let (_, thumb) = metrics(&grid).vertical_scrollbar().unwrap();
        let start = inside(thumb);

        press(&mut grid, start);
        grid.pointer_moved(&Fixture, Point::new(start.x, start.y + 40.0), viewport());
        grid.pointer_released(&hooks());

        let expected = Metrics::new(Vector::new(0.0, 0.0), viewport(), Fixture.dims())
            .thumb_drag(Axis::Vertical, 40.0);
        assert_eq!(grid.scroll.y, expected);
        assert_eq!(grid.selection, Selection::single(CellRef::new(0, 0)), "no cell was picked");
    }

    #[test]
    fn a_press_on_the_bar_never_reaches_the_cell_behind_it() {
        let mut grid = GridController::new();
        let (track, _) = metrics(&grid).vertical_scrollbar().unwrap();
        let position = Point::new(track.x + 2.0, 200.0);
        assert!(metrics(&grid).cell_at(position).is_some(), "a cell really is underneath");

        press(&mut grid, position);
        assert_eq!(grid.selection, Selection::single(CellRef::new(0, 0)));
    }

    #[test]
    fn clicking_the_empty_track_pages_the_view() {
        let mut grid = GridController::new();
        let (track, thumb) = metrics(&grid).vertical_scrollbar().unwrap();

        press(&mut grid, Point::new(track.x + 2.0, thumb.y + thumb.height + 30.0));

        assert_eq!(grid.scroll.y, metrics(&grid).grid_size().height);
        assert_eq!(grid.drag, None, "paging is a click, not a drag");
    }

    #[test]
    fn wheel_scrolling_is_clamped_and_uses_the_viewport_it_was_given() {
        let mut grid = GridController::new();
        let event = GridEvent::Scrolled { delta: Vector::new(0.0, 120.0), viewport: viewport() };
        grid.handle(&Fixture, &event, false, Instant::now(), &hooks());
        assert_eq!(grid.scroll.y, 120.0);

        let back = GridEvent::Scrolled { delta: Vector::new(0.0, -1000.0), viewport: viewport() };
        grid.handle(&Fixture, &back, false, Instant::now(), &hooks());
        assert_eq!(grid.scroll, Vector::new(0.0, 0.0), "never scrolls past the origin");
    }

    #[test]
    fn clearing_is_the_hosts_job_but_the_controller_says_what_to_clear() {
        let mut grid = GridController::new();
        grid.select_row(&Fixture, 3);
        assert_eq!(
            grid.request_clear(&hooks()),
            vec![Asked::Clear(Bounds::new(CellRef::new(3, 0), CellRef::new(3, 39)))]
        );
    }
}
