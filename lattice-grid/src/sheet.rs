//! The widget itself: the canvas program, the events it sends back, and the
//! geometry it paints with.
//!
//! [`GridProgram`] is what goes inside an iced canvas. It draws from a
//! [`SheetModel`] and reports everything the pointer does as a [`GridEvent`].
//! [`Metrics`] is the layout behind both, and it's public on purpose: a host
//! doing its own hit-testing can go through it and land on the same pixels the
//! grid draws with, instead of guessing at the numbers separately.

use iced_core::{
    alignment, mouse, window, Font, Pixels, Point, Rectangle, Size, Theme, Vector,
};
use iced_widget::Renderer;
use iced_widget::canvas::{self, Action, Frame, Geometry, LineCap, Path, Stroke, Text};

use crate::model::{col_name, format_number, Bounds, CellRef, CellValue, Dims, SheetModel};
use crate::style::{GardenPalette, FOCUS_BORDER, HAIRLINE};

/// The width of the row gutter down the left edge, in pixels.
pub const HEADER_WIDTH: f32 = 54.0;
/// The height of the column gutter across the top, in pixels.
pub const HEADER_HEIGHT: f32 = 24.0;
/// The width of one cell, in pixels.
///
/// Cells are a fixed size rather than sized to their content, which is what
/// lets the grid work out what to paint without measuring anything.
pub const CELL_WIDTH: f32 = 104.0;
/// The height of one cell, in pixels.
pub const CELL_HEIGHT: f32 = 26.0;
/// How many rows and columns past the visible edge get painted.
///
/// A little slack keeps a half-scrolled row from popping in as you scroll, at
/// the cost of painting a couple of lines that aren't on screen yet.
pub const BUFFER: u32 = 1;
/// The side of the fill handle at the bottom-right of a selection, in pixels.
///
/// The square that gets drawn is this size. The area that responds to a press
/// is a few pixels larger, so it isn't fiddly to grab.
pub const FILL_HANDLE: f32 = 7.0;
/// The thickness of a scrollbar, in pixels.
pub const SCROLLBAR: f32 = 12.0;

const FONT_SIZE: f32 = 13.0;
const TEXT_PADDING: f32 = 7.0;

/// A scroll axis, for the calls that work the same way in both directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    /// The vertical axis, along the right-hand scrollbar.
    Vertical,
    /// The horizontal axis, along the bottom scrollbar.
    Horizontal,
}

impl Axis {
    /// The point's coordinate along this axis: `y` for vertical, `x` for
    /// horizontal.
    pub fn along(self, point: Point) -> f32 {
        match self {
            Axis::Vertical => point.y,
            Axis::Horizontal => point.x,
        }
    }

    /// The scroll offset's component along this axis.
    ///
    /// The other component is simply ignored, so a vertical call on a
    /// half-scrolled sheet reads only the vertical part.
    pub fn of(self, scroll: Vector) -> f32 {
        match self {
            Axis::Vertical => scroll.y,
            Axis::Horizontal => scroll.x,
        }
    }

    /// `scroll` with its component along this axis replaced by `value`.
    ///
    /// The other component is carried through untouched, which is what keeps a
    /// vertical scroll from resetting the horizontal one.
    pub fn with(self, scroll: Vector, value: f32) -> Vector {
        match self {
            Axis::Vertical => Vector::new(scroll.x, value),
            Axis::Horizontal => Vector::new(value, scroll.y),
        }
    }

    fn length(self, size: Size) -> f32 {
        match self {
            Axis::Vertical => size.height,
            Axis::Horizontal => size.width,
        }
    }
}

/// Which part of a scrollbar a point landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarHit {
    /// The thumb itself. A host would start dragging it.
    Thumb(Axis),
    /// The empty track beside the thumb.
    Track {
        /// The bar that was hit.
        axis: Axis,
        /// `true` when the point is past the thumb, towards the end of the
        /// sheet. That's the direction to page in.
        forward: bool,
    },
}

/// The geometry of the grid at one viewport size and scroll offset.
///
/// Everything on it is worked out from the three fields, so it's cheap to build
/// once a frame and cheap to throw away. It is the same arithmetic the grid
/// paints with, which is why a host doing its own hit-testing should go through
/// it rather than repeat the sums: that's how a click ends up one column over.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// The scroll offset in pixels, measured from the sheet's top-left corner.
    pub scroll: Vector,
    /// The size of the whole canvas, gutters and scrollbars included.
    pub viewport: Size,
    /// The sheet's extent, as the model reports it. Every bound below follows
    /// from this rather than from a fixed assumption about sheet size.
    pub dims: Dims,
}

impl Metrics {
    /// The geometry for one frame.
    ///
    /// `viewport` is the canvas size, not the window's, so a host with chrome
    /// around the grid should pass what's left after the chrome.
    pub fn new(scroll: Vector, viewport: Size, dims: Dims) -> Self {
        Metrics { scroll, viewport, dims }
    }

    /// The size of the cell area: the viewport with the gutters taken off.
    ///
    /// Scrollbars aren't subtracted, because they're drawn over the last row and
    /// column rather than laid out beside them.
    pub fn grid_size(&self) -> Size {
        Size::new(
            (self.viewport.width - HEADER_WIDTH).max(0.0),
            (self.viewport.height - HEADER_HEIGHT).max(0.0),
        )
    }

    /// The rows a frame should paint, as a half-open range.
    ///
    /// It's inset by [`BUFFER`] at each end, so it reaches a row or two above
    /// and below the viewport. Clamped to the sheet, so a sheet smaller than the
    /// viewport gives a range covering exactly that sheet.
    pub fn visible_rows(&self) -> std::ops::Range<u32> {
        let first = (self.scroll.y.max(0.0) / CELL_HEIGHT).floor() as u32;
        let first = first.saturating_sub(BUFFER);
        let span = (self.grid_size().height / CELL_HEIGHT).ceil() as u32 + 1 + 2 * BUFFER;
        let last = first.saturating_add(span).min(self.dims.rows);
        first..last.max(first + 1).min(self.dims.rows)
    }

    /// The columns a frame should paint, as a half-open range.
    ///
    /// The horizontal counterpart of [`Metrics::visible_rows`], with the same
    /// [`BUFFER`] of slack at each end.
    pub fn visible_cols(&self) -> std::ops::Range<u32> {
        let first = (self.scroll.x.max(0.0) / CELL_WIDTH).floor() as u32;
        let first = first.saturating_sub(BUFFER);
        let span = (self.grid_size().width / CELL_WIDTH).ceil() as u32 + 1 + 2 * BUFFER;
        let last = first.saturating_add(span).min(self.dims.cols);
        first..last.max(first + 1).min(self.dims.cols)
    }

    /// Where a cell is on screen, in canvas-local coordinates.
    ///
    /// A cell that's scrolled off is still reported, so the result can fall
    /// partly or wholly outside the viewport. A caller that only wants what's
    /// visible should check the rectangle against the viewport, the way the
    /// grid's own painting does.
    pub fn cell_rect(&self, cell: CellRef) -> Rectangle {
        Rectangle::new(
            Point::new(
                HEADER_WIDTH + cell.col as f32 * CELL_WIDTH - self.scroll.x,
                HEADER_HEIGHT + cell.row as f32 * CELL_HEIGHT - self.scroll.y,
            ),
            Size::new(CELL_WIDTH, CELL_HEIGHT),
        )
    }

    /// Where a whole block is on screen.
    ///
    /// The result covers both corners whatever order they're in, since
    /// [`Bounds`] is already normalised. This is the rectangle to sit an editor
    /// or an overlay on when it should span a selection.
    pub fn bounds_rect(&self, bounds: Bounds) -> Rectangle {
        let top_left = self.cell_rect(CellRef::new(bounds.min_row, bounds.min_col));
        let bottom_right = self.cell_rect(CellRef::new(bounds.max_row, bounds.max_col));
        Rectangle::new(
            top_left.position(),
            Size::new(
                bottom_right.x + bottom_right.width - top_left.x,
                bottom_right.y + bottom_right.height - top_left.y,
            ),
        )
    }

    /// The cell under a point, or `None` if there isn't one.
    ///
    /// `None` covers all four ways a press can miss: a gutter, above or left of
    /// the sheet, and past the sheet's own `dims` at the far edge. The inverse
    /// of [`Metrics::cell_rect`].
    ///
    /// ```
    /// use iced_core::{Size, Vector};
    /// use lattice_grid::{CellRef, Dims, Metrics};
    ///
    /// let metrics = Metrics::new(Vector::new(0.0, 0.0), Size::new(900.0, 500.0), Dims { rows: 10, cols: 10 });
    ///
    /// // 54px of gutter, then 104px cells, so the second cell starts at 158.
    /// assert_eq!(metrics.cell_at(iced_core::Point::new(160.0, 30.0)), Some(CellRef::new(0, 1)));
    /// assert_eq!(metrics.cell_at(iced_core::Point::new(10.0, 30.0)), None);
    /// ```
    pub fn cell_at(&self, position: Point) -> Option<CellRef> {
        if position.x < HEADER_WIDTH || position.y < HEADER_HEIGHT {
            return None;
        }
        let x = position.x - HEADER_WIDTH + self.scroll.x;
        let y = position.y - HEADER_HEIGHT + self.scroll.y;
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let col = (x / CELL_WIDTH) as u32;
        let row = (y / CELL_HEIGHT) as u32;
        if row >= self.dims.rows || col >= self.dims.cols {
            return None;
        }
        Some(CellRef::new(row, col))
    }

    /// Where a row's gutter label sits on screen.
    ///
    /// The rectangle is only as tall as one cell, so a host drawing something
    /// down the whole gutter has to use it per row.
    pub fn row_header_rect(&self, row: u32) -> Rectangle {
        Rectangle::new(
            Point::new(0.0, HEADER_HEIGHT + row as f32 * CELL_HEIGHT - self.scroll.y),
            Size::new(HEADER_WIDTH, CELL_HEIGHT),
        )
    }

    /// Where a column's gutter label sits on screen.
    ///
    /// The rectangle is one cell wide and [`HEADER_HEIGHT`] tall.
    pub fn col_header_rect(&self, col: u32) -> Rectangle {
        Rectangle::new(
            Point::new(HEADER_WIDTH + col as f32 * CELL_WIDTH - self.scroll.x, 0.0),
            Size::new(CELL_WIDTH, HEADER_HEIGHT),
        )
    }

    /// The little square at the bottom-right of a selection that a fill drag
    /// starts from.
    ///
    /// It's centred on the corner cell's bottom-right, so half of it sits
    /// outside the selection.
    pub fn fill_handle(&self, selection: Bounds) -> Rectangle {
        let corner = self.cell_rect(CellRef::new(selection.max_row, selection.max_col));
        Rectangle::new(
            Point::new(
                corner.x + corner.width - FILL_HANDLE / 2.0,
                corner.y + corner.height - FILL_HANDLE / 2.0,
            ),
            Size::new(FILL_HANDLE, FILL_HANDLE),
        )
    }

    /// Whether a press landed on the fill handle.
    ///
    /// The test area is a few pixels larger than the drawn square, so the handle
    /// is easier to grab than it looks. Check this before [`Metrics::cell_at`],
    /// or a press on the handle reads as a press on the cell underneath it.
    pub fn hits_fill_handle(&self, selection: Bounds, position: Point) -> bool {
        let handle = self.fill_handle(selection);
        let slack = 4.0;
        Rectangle::new(
            Point::new(handle.x - slack, handle.y - slack),
            Size::new(handle.width + 2.0 * slack, handle.height + 2.0 * slack),
        )
        .expand(1.0)
        .contains(position)
    }

    /// `scroll` pulled back inside the sheet.
    ///
    /// Never negative, and never far enough to leave a gap past the last row or
    /// column. On an axis where the sheet is smaller than the viewport, the
    /// offset pins to zero, which is why a small sheet has no scrollbars.
    ///
    /// ```
    /// use iced_core::{Size, Vector};
    /// use lattice_grid::{Dims, Metrics};
    ///
    /// let dims = Dims { rows: 100, cols: 10 };
    /// let metrics = Metrics::new(Vector::new(0.0, 0.0), Size::new(900.0, 500.0), dims);
    ///
    /// // Nothing above or left of the sheet.
    /// assert_eq!(metrics.clamp_scroll(Vector::new(-50.0, -50.0)), Vector::new(0.0, 0.0));
    /// ```
    pub fn clamp_scroll(&self, scroll: Vector) -> Vector {
        Vector::new(
            scroll.x.clamp(0.0, Scrolling::extent(self.dims.cols as f32 * CELL_WIDTH, self.grid_size().width)),
            scroll.y.clamp(0.0, Scrolling::extent(self.dims.rows as f32 * CELL_HEIGHT, self.grid_size().height)),
        )
    }

    /// The offset that brings a cell fully into view, moving as little as
    /// possible.
    ///
    /// A cell already on screen leaves `scroll` alone. The result is already
    /// clamped, so it's safe to assign straight to `GridController::scroll`.
    /// This is what keyboard navigation uses to follow the active cell.
    pub fn scroll_to_show(&self, cell: CellRef, scroll: Vector) -> Vector {
        let grid = self.grid_size();
        let mut next = scroll;

        let left = cell.col as f32 * CELL_WIDTH;
        let right = left + CELL_WIDTH;
        if left < next.x {
            next.x = left;
        } else if right > next.x + grid.width {
            next.x = right - grid.width;
        }

        let top = cell.row as f32 * CELL_HEIGHT;
        let bottom = top + CELL_HEIGHT;
        if top < next.y {
            next.y = top;
        } else if bottom > next.y + grid.height {
            next.y = bottom - grid.height;
        }

        self.clamp_scroll(next)
    }

    /// The track and thumb rectangles for one axis.
    ///
    /// `None` when the sheet fits on that axis and there's nothing to scroll, so
    /// a host painting its own bars should treat `None` as "draw nothing".
    pub fn scrollbar(&self, axis: Axis) -> Option<(Rectangle, Rectangle)> {
        let (track, content, offset) = match axis {
            Axis::Vertical => (
                Rectangle::new(
                    Point::new(self.viewport.width - SCROLLBAR, HEADER_HEIGHT),
                    Size::new(SCROLLBAR, (self.viewport.height - HEADER_HEIGHT).max(0.0)),
                ),
                self.dims.rows as f32 * CELL_HEIGHT,
                self.scroll.y,
            ),
            Axis::Horizontal => (
                Rectangle::new(
                    Point::new(HEADER_WIDTH, self.viewport.height - SCROLLBAR),
                    Size::new((self.viewport.width - HEADER_WIDTH).max(0.0), SCROLLBAR),
                ),
                self.dims.cols as f32 * CELL_WIDTH,
                self.scroll.x,
            ),
        };
        Scrolling::thumb(track, content, offset)
    }

    /// The right-hand scrollbar as a track and a thumb.
    ///
    /// `None` when the sheet is shorter than the viewport.
    pub fn vertical_scrollbar(&self) -> Option<(Rectangle, Rectangle)> {
        self.scrollbar(Axis::Vertical)
    }

    /// The bottom scrollbar as a track and a thumb.
    ///
    /// `None` when the sheet is narrower than the viewport.
    pub fn horizontal_scrollbar(&self) -> Option<(Rectangle, Rectangle)> {
        self.scrollbar(Axis::Horizontal)
    }

    /// Which part of which scrollbar a point landed on, or `None` if it missed
    /// both.
    ///
    /// The horizontal bar is tested first, so the corner where the two bars meet
    /// belongs to it. A host should ask this before [`Metrics::cell_at`]: the
    /// bars are painted over the last row and column, so a point on a bar is
    /// also a point on a cell behind it.
    pub fn scrollbar_at(&self, position: Point) -> Option<ScrollbarHit> {
        // Horizontal first: painted last, so it owns the corner
        for axis in [Axis::Horizontal, Axis::Vertical] {
            let Some((track, thumb)) = self.scrollbar(axis) else {
                continue;
            };
            if !track.contains(position) {
                continue;
            }
            if thumb.contains(position) {
                return Some(ScrollbarHit::Thumb(axis));
            }
            let forward = axis.along(position) > axis.along(thumb.position());
            return Some(ScrollbarHit::Track { axis, forward });
        }
        None
    }

    /// Turns a drag in pixels into a scroll delta for that axis.
    ///
    /// The conversion uses the same ratio the drawn thumb moves at, so the thumb
    /// stays under the pointer as it's dragged. Returns zero when there's
    /// nothing to scroll, or when the thumb fills the whole track.
    pub fn thumb_drag(&self, axis: Axis, delta: f32) -> f32 {
        let Some((track, thumb)) = self.scrollbar(axis) else {
            return 0.0;
        };
        let span = axis.length(track.size());
        let travel = span - axis.length(thumb.size());
        if travel <= 0.0 {
            return 0.0;
        }
        delta * Scrolling::extent(self.content_extent(axis), span) / travel
    }

    /// How far a click on the empty track should move the view.
    ///
    /// Roughly one viewport, so paging twice lands where the old screen ended.
    /// Negative when paging back. Add it to the current offset for that axis and
    /// clamp, rather than assigning it directly.
    pub fn page_scroll(&self, axis: Axis, forward: bool) -> f32 {
        let page = axis.length(self.grid_size());
        if forward { page } else { -page }
    }

    fn content_extent(&self, axis: Axis) -> f32 {
        match axis {
            Axis::Vertical => self.dims.rows as f32 * CELL_HEIGHT,
            Axis::Horizontal => self.dims.cols as f32 * CELL_WIDTH,
        }
    }
}

struct Scrolling;

impl Scrolling {
    fn extent(content: f32, viewport: f32) -> f32 {
        (content - viewport).max(0.0)
    }

    fn thumb(track: Rectangle, content: f32, offset: f32) -> Option<(Rectangle, Rectangle)> {
        let span = if track.width > track.height { track.width } else { track.height };
        if content <= span {
            return None;
        }
        let ratio = (span / content).clamp(0.02, 1.0);
        let length = span * ratio;
        let progress = (offset / Self::extent(content, span)).clamp(0.0, 1.0);
        let thumb = if track.width > track.height {
            Rectangle::new(
                Point::new(track.x + progress * (span - length), track.y),
                Size::new(length, track.height),
            )
        } else {
            Rectangle::new(
                Point::new(track.x, track.y + progress * (span - length)),
                Size::new(track.width, length),
            )
        };
        Some((track, thumb))
    }
}

/// Cuts text down to what fits a width, with an ellipsis when it has to.
///
/// The fit is estimated from an average character width rather than measured,
/// because canvas text is drawn without clipping. It's a good guess rather than
/// an exact one, so a run of wide letters can still overhang a little.
///
/// ```
/// assert_eq!(lattice_grid::fit_text("hello", 100.0, 13.0), "hello");
/// assert_eq!(lattice_grid::fit_text("hello world", 40.0, 13.0), "hell…");
/// ```
pub fn fit_text(text: &str, max_width: f32, font_size: f32) -> String {
    let capacity = characters_that_fit(max_width, font_size);
    if text.chars().count() <= capacity {
        return text.to_string();
    }
    let mut out: String = text.chars().take(capacity.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn characters_that_fit(max_width: f32, font_size: f32) -> usize {
    let advance = font_size * 0.58;
    (max_width / advance).floor().max(1.0) as usize
}

/// A number as it paints in a cell: [`format_number`] when it fits, and
/// rounded to fewer decimals until it does.
///
/// Note the difference from [`fit_text`]: a number that's too wide is rounded,
/// not cut off, so a cell never shows an ellipsis where a host expects a value.
/// Only when even zero decimals are still too wide does it fall back to
/// truncating.
///
/// ```
/// use lattice_grid::fit_number;
///
/// // Room for it, so it reads as it would anywhere else.
/// assert_eq!(fit_number(1234.0, 400.0, 13.0), "1234");
///
/// // A narrow column rounds it instead of cutting it off.
/// assert_eq!(fit_number(1.0 / 3.0, 40.0, 13.0), "0.333");
/// ```
pub fn fit_number(value: f64, max_width: f32, font_size: f32) -> String {
    let capacity = characters_that_fit(max_width, font_size);
    let full = format_number(value);
    if full.chars().count() <= capacity {
        return full;
    }
    for decimals in (0..=8).rev() {
        let candidate = format!("{value:.decimals$}");
        if candidate.chars().count() <= capacity {
            return candidate;
        }
    }
    fit_text(&full, max_width, font_size)
}

/// What the grid reports to its host. The widget never names the host's
/// messages: it emits these, and the host decides what they mean. That is what
/// lets the same grid live in someone else's application.
#[derive(Clone, Debug, PartialEq)]
pub enum GridEvent {
    /// A left button went down.
    PointerPressed {
        /// Where the pointer is, in canvas-local coordinates. Hit-test it with
        /// [`Metrics`].
        position: Point,
        /// The canvas size, so the host hit-tests against the same viewport the
        /// grid painted with.
        viewport: Size,
    },
    /// The left button came up. Commit whatever drag was in flight.
    PointerReleased,
    /// The pointer moved.
    PointerMoved {
        /// Where the pointer is now. It can be outside the canvas: a move is
        /// still reported once a drag has left the grid, which is what lets a
        /// drag keep extending off the edge.
        position: Point,
        /// The canvas size.
        viewport: Size,
    },
    /// A wheel or trackpad scroll.
    Scrolled {
        /// How far to scroll, in pixels. Line-based deltas are already converted
        /// to pixels. Add this to the current offset and clamp the result,
        /// rather than assigning it outright.
        delta: Vector,
        /// The canvas size, since the clamp depends on it.
        viewport: Size,
    },
    /// The window was resized.
    ///
    /// The size is the canvas's own bounds, the same rectangle the position
    /// fields on the other events are measured against, rather than the
    /// window's. It arrives before that frame's pointer events, so a host can
    /// re-derive its viewport and re-clamp its scroll before the next click
    /// lands.
    Viewport(Size),
}

/// The canvas program to put inside an iced canvas.
///
/// It paints the grid from a [`SheetModel`] and reports everything the pointer
/// does through `on_event`. It keeps no state of its own: the host holds the
/// selection, the scroll offset and the rest, and re-rendering with the same
/// fields paints the same frame.
///
/// ```
/// use iced_core::Vector;
/// use iced_widget::canvas;
/// use lattice_grid::{
///     Bounds, CellRef, CellValue, Dims, GardenPalette, GridEvent, GridProgram, SheetModel,
/// };
///
/// struct Data;
///
/// impl SheetModel for Data {
///     fn dims(&self) -> Dims {
///         Dims { rows: 4, cols: 4 }
///     }
///
///     fn value(&self, _cell: CellRef) -> CellValue {
///         CellValue::Empty
///     }
/// }
///
/// #[derive(Debug, Clone)]
/// enum Message {
///     /// The grid's events, forwarded into the host's own message type.
///     Grid(GridEvent),
/// }
///
/// let _program = canvas(GridProgram {
///     model: Data,
///     selection: Bounds::single(CellRef::new(0, 0)),
///     active: CellRef::new(0, 0),
///     fill_preview: None,
///     scroll: Vector::new(0.0, 0.0),
///     active_scrollbar: None,
///     palette: GardenPalette::light(),
///     on_event: Message::Grid,
/// });
/// ```
pub struct GridProgram<M, S: SheetModel> {
    /// The host's data. Held by value: the host builds it where it builds the
    /// rest of the widget, and the grid only ever asks it for values.
    pub model: S,
    /// The block to highlight. The grid normalises it, so a drag that went
    /// backwards is fine.
    pub selection: Bounds,
    /// The cell an editor would open on, outlined more heavily than the rest of
    /// the selection. This is usually `GridController::selection.active`.
    pub active: CellRef,
    /// The block a fill drag is about to write, drawn in the stronger fill.
    /// `None` unless a fill is in progress.
    pub fill_preview: Option<Bounds>,
    /// The scroll offset in pixels. Clamp it with [`Metrics::clamp_scroll`].
    pub scroll: Vector,
    /// The scrollbar currently held, drawn in its active colour. `None` when no
    /// bar is being dragged.
    pub active_scrollbar: Option<Axis>,
    /// The grid's colours. Passed in rather than read from the iced theme,
    /// because a theme carries six colours and the grid uses rather more.
    pub palette: GardenPalette,
    /// Maps a grid event onto the host's message type.
    ///
    /// A plain function pointer, so the grid never has to be generic over
    /// messages it doesn't understand.
    pub on_event: fn(GridEvent) -> M,
}

impl<M, S: SheetModel> GridProgram<M, S> {
    fn metrics(&self, bounds: Rectangle) -> Metrics {
        Metrics::new(self.scroll, bounds.size(), self.model.dims())
    }
}

impl<M, S: SheetModel> canvas::Program<M> for GridProgram<M, S> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<M>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // Cursor positions are window-absolute; canvas works in local coords
                let position = cursor.position_in(bounds)?;
                Some(Action::publish((self.on_event)(GridEvent::PointerPressed { position, viewport: bounds.size() })).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Some(Action::publish((self.on_event)(GridEvent::PointerReleased)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // Deliberately unclipped so drags leaving the grid keep extending
                let position = cursor.position_from(bounds.position())?;
                Some(Action::publish((self.on_event)(GridEvent::PointerMoved { position, viewport: bounds.size() })))
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let delta = match delta {
                    mouse::ScrollDelta::Lines { x, y } => Vector::new(x * 48.0, y * 48.0),
                    mouse::ScrollDelta::Pixels { x, y } => Vector::new(*x, *y),
                };
                Some(Action::publish((self.on_event)(GridEvent::Scrolled { delta, viewport: bounds.size() })).and_capture())
            }
            // Report the canvas's bounds, not the window's: hosts hit-test in
            // canvas coordinates, and every other event on this enum uses those
            canvas::Event::Window(window::Event::Resized(_)) => {
                Some(Action::publish((self.on_event)(GridEvent::Viewport(bounds.size()))))
            }
            _ => None,
        }
    }

    fn mouse_interaction(
        &self,
        _state: &(),
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        let Some(position) = cursor.position_in(bounds) else {
            return mouse::Interaction::default();
        };
        let metrics = self.metrics(bounds);
        if metrics.hits_fill_handle(self.selection, position) {
            mouse::Interaction::Crosshair
        } else if let Some(ScrollbarHit::Thumb(_)) = metrics.scrollbar_at(position) {
            mouse::Interaction::Grab
        } else if metrics.scrollbar_at(position).is_some() {
            mouse::Interaction::default()
        } else if metrics.cell_at(position).is_some() {
            mouse::Interaction::Text
        } else {
            mouse::Interaction::default()
        }
    }

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let metrics = self.metrics(bounds);
        let mut frame = Frame::new(renderer, bounds.size());

        frame.fill_rectangle(Point::ORIGIN, bounds.size(), self.palette.canvas);

        let selected = metrics.bounds_rect(self.selection);
        frame.fill_rectangle(
            Point::new(selected.x.max(HEADER_WIDTH), selected.y.max(HEADER_HEIGHT)),
            Size::new(
                (selected.width - (HEADER_WIDTH - selected.x).max(0.0)).max(0.0),
                (selected.height - (HEADER_HEIGHT - selected.y).max(0.0)).max(0.0),
            ),
            self.palette.selection_fill,
        );

        let rows = metrics.visible_rows();
        let cols = metrics.visible_cols();
        let hairline = Stroke::default().with_width(HAIRLINE).with_color(self.palette.lattice);

        for row in rows.clone() {
            let rect = metrics.cell_rect(CellRef::new(row, 0));
            if rect.y < HEADER_HEIGHT {
                continue;
            }
            frame.stroke(
                &Path::line(
                    Point::new(HEADER_WIDTH, rect.y),
                    Point::new(bounds.width, rect.y),
                ),
                hairline,
            );
        }
        for col in cols.clone() {
            let rect = metrics.cell_rect(CellRef::new(0, col));
            if rect.x < HEADER_WIDTH {
                continue;
            }
            frame.stroke(
                &Path::line(Point::new(rect.x, HEADER_HEIGHT), Point::new(rect.x, bounds.height)),
                hairline,
            );
        }

        for row in rows {
            for col in cols.clone() {
                let cell = CellRef::new(row, col);
                let value = self.model.value(cell);
                if value.is_empty() {
                    continue;
                }
                let rect = metrics.cell_rect(cell);
                if rect.x + rect.width < HEADER_WIDTH || rect.y + rect.height < HEADER_HEIGHT {
                    continue;
                }
                let align = value.alignment();
                let text = match &value {
                    CellValue::Number(n) => fit_number(*n, rect.width - 2.0 * TEXT_PADDING, FONT_SIZE),
                    other => fit_text(&other.as_text(), rect.width - 2.0 * TEXT_PADDING, FONT_SIZE),
                };
                let color = if value.is_error() { self.palette.clay } else { self.palette.ink };
                let available = rect.width - 2.0 * TEXT_PADDING;
                let x = match align {
                    alignment::Horizontal::Right => rect.x + rect.width - TEXT_PADDING,
                    _ => rect.x + TEXT_PADDING,
                };
                frame.fill_text(Text {
                    content: text,
                    position: Point::new(x, rect.y + rect.height / 2.0),
                    color,
                    size: Pixels(FONT_SIZE),
                    font: Font::DEFAULT,
                    align_x: align.into(),
                    align_y: alignment::Vertical::Center,
                    max_width: available,
                    ..Text::default()
                });
            }
        }

        self.draw_gutters(&mut frame, &metrics, bounds, &selected);

        if let Some(preview) = self.fill_preview {
            let rect = metrics.bounds_rect(preview);
            frame.fill_rectangle(
                rect.position(),
                rect.size(),
                self.palette.selection_fill_active,
            );
            frame.stroke_rectangle(
                rect.position(),
                rect.size(),
                Stroke::default().with_width(FOCUS_BORDER).with_color(self.palette.leaf),
            );
        }
        frame.stroke_rectangle(
            metrics.bounds_rect(self.selection).position(),
            metrics.bounds_rect(self.selection).size(),
            Stroke::default().with_width(FOCUS_BORDER).with_color(self.palette.leaf),
        );

        let handle = metrics.fill_handle(self.selection);
        frame.fill_rectangle(
            Point::new(handle.x, handle.y),
            handle.size(),
            self.palette.leaf,
        );
        frame.stroke(
            &Path::rectangle(Point::new(handle.x, handle.y), handle.size()),
            Stroke::default().with_width(1.0).with_color(self.palette.canvas),
        );

        for (axis, bar) in [
            (Axis::Vertical, metrics.vertical_scrollbar()),
            (Axis::Horizontal, metrics.horizontal_scrollbar()),
        ] {
            let Some((track, thumb)) = bar else { continue };
            frame.fill_rectangle(track.position(), track.size(), self.palette.scrollbar_track);
            // A held thumb darkens so the grab is unmistakable
            let colour = if self.active_scrollbar == Some(axis) {
                self.palette.scrollbar_thumb_active
            } else {
                self.palette.scrollbar_thumb
            };
            frame.fill_rectangle(thumb.position(), thumb.size(), colour);
        }

        vec![frame.into_geometry()]
    }
}

impl<M, S: SheetModel> GridProgram<M, S> {
    fn draw_gutters(
        &self,
        frame: &mut Frame<Renderer>,
        metrics: &Metrics,
        bounds: Rectangle,
        selected: &Rectangle,
    ) {
        let rows = metrics.visible_rows();
        let cols = metrics.visible_cols();
        let selection = self.selection;

        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(bounds.width, HEADER_HEIGHT),
            self.palette.surface,
        );
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(HEADER_WIDTH, bounds.height),
            self.palette.surface,
        );

        for row in rows.clone() {
            if row < selection.min_row || row > selection.max_row {
                continue;
            }
            let rect = metrics.row_header_rect(row);
            if rect.y < HEADER_HEIGHT {
                continue;
            }
            frame.fill_rectangle(rect.position(), rect.size(), self.palette.gutter_active);
        }
        for col in cols.clone() {
            if col < selection.min_col || col > selection.max_col {
                continue;
            }
            let rect = metrics.col_header_rect(col);
            if rect.x < HEADER_WIDTH {
                continue;
            }
            frame.fill_rectangle(rect.position(), rect.size(), self.palette.gutter_active);
        }

        for row in rows {
            let rect = metrics.row_header_rect(row);
            if rect.y < HEADER_HEIGHT {
                continue;
            }
            let label = (row + 1).to_string();
            frame.fill_text(Text {
                content: label,
                position: Point::new(HEADER_WIDTH - TEXT_PADDING, rect.y + rect.height / 2.0),
                color: self.palette.ink_soft,
                size: Pixels(FONT_SIZE - 1.0),
                align_x: alignment::Horizontal::Right.into(),
                align_y: alignment::Vertical::Center,
                ..Text::default()
            });
        }
        for col in cols {
            let rect = metrics.col_header_rect(col);
            if rect.x < HEADER_WIDTH {
                continue;
            }
            let label = col_name(col);
            frame.fill_text(Text {
                content: label,
                position: Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
                color: self.palette.ink_soft,
                size: Pixels(FONT_SIZE - 1.0),
                align_x: alignment::Horizontal::Center.into(),
                align_y: alignment::Vertical::Center,
                ..Text::default()
            });
        }

        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(HEADER_WIDTH, HEADER_HEIGHT),
            self.palette.surface_deep,
        );
        let line = Stroke::default().with_width(1.0).with_color(self.palette.lattice_strong);
        frame.stroke(
            &Path::line(Point::new(0.0, HEADER_HEIGHT), Point::new(bounds.width, HEADER_HEIGHT)),
            line,
        );
        frame.stroke(
            &Path::line(Point::new(HEADER_WIDTH, 0.0), Point::new(HEADER_WIDTH, bounds.height)),
            line,
        );

        let row_rect = metrics.row_header_rect(self.active.row);
        if row_rect.y >= HEADER_HEIGHT {
            frame.fill_rectangle(
                Point::new(0.0, row_rect.y),
                Size::new(HEADER_WIDTH, row_rect.height),
                self.palette.selection_fill_active,
            );
        }
        let col_rect = metrics.col_header_rect(self.active.col);
        if col_rect.x >= HEADER_WIDTH {
            frame.fill_rectangle(
                col_rect.position(),
                Size::new(col_rect.width, HEADER_HEIGHT),
                self.palette.selection_fill_active,
            );
        }

        let active_rect = metrics.bounds_rect(Bounds::single(self.active));
        if active_rect.y >= HEADER_HEIGHT && active_rect.x >= HEADER_WIDTH {
            let outline = Stroke::default().with_width(FOCUS_BORDER).with_color(self.palette.leaf);
            let top_left = Point::new(active_rect.x, active_rect.y);
            let bottom_right = Point::new(
                active_rect.x + active_rect.width,
                active_rect.y + active_rect.height,
            );
            frame.stroke(
                &Path::new(|builder| {
                    builder.move_to(Point::new(top_left.x, top_left.y));
                    builder.line_to(Point::new(bottom_right.x, top_left.y));
                    builder.line_to(bottom_right);
                    builder.line_to(Point::new(top_left.x, bottom_right.y));
                    builder.close();
                }),
                outline,
            );
        }
        let _ = LineCap::Butt;
        let _ = selected;
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    fn metrics() -> Metrics {
        Metrics::new(Vector::new(0.0, 0.0), Size::new(1000.0, 700.0), Dims::SPREADSHEET)
    }

    // Half way along both tracks: room on both sides
    fn mid_scrolled() -> Metrics {
        let m = metrics();
        let grid = m.grid_size();
        let half = Vector::new(
            Scrolling::extent(Dims::SPREADSHEET.cols as f32 * CELL_WIDTH, grid.width) / 2.0,
            Scrolling::extent(Dims::SPREADSHEET.rows as f32 * CELL_HEIGHT, grid.height) / 2.0,
        );
        Metrics::new(m.clamp_scroll(half), m.viewport, Dims::SPREADSHEET)
    }

    #[test]
    fn visible_ranges_cover_the_viewport_without_touching_the_sheet_size() {
        let m = metrics();
        let rows = m.visible_rows();
        let cols = m.visible_cols();
        assert!(rows.start == 0);
        assert!(rows.end >= 27 && rows.end <= 31, "{rows:?}");
        assert!(cols.end >= 10 && cols.end <= 13, "{cols:?}");
        assert!(cols.end < Dims::SPREADSHEET.cols);
    }

    #[test]
    fn scrolling_moves_the_visible_window() {
        let mut m = metrics();
        m.scroll = Vector::new(0.0, CELL_HEIGHT * 100.0);
        let rows = m.visible_rows();
        assert_eq!(rows.start, 99);
        assert!(rows.contains(&100));

        m.scroll = Vector::new(CELL_WIDTH * 50.0, 0.0);
        let cols = m.visible_cols();
        assert_eq!(cols.start, 49);
        assert!(cols.contains(&50));
    }

    #[test]
    fn the_visible_window_stays_inside_the_sheet_at_the_far_end() {
        let mut m = metrics();
        m.scroll = Vector::new(Dims::SPREADSHEET.cols as f32 * CELL_WIDTH, Dims::SPREADSHEET.rows as f32 * CELL_HEIGHT);
        let rows = m.visible_rows();
        let cols = m.visible_cols();
        assert!(rows.end <= Dims::SPREADSHEET.rows);
        assert!(cols.end <= Dims::SPREADSHEET.cols);
        assert!(rows.start < rows.end, "range must not be empty");
        assert!(cols.start < cols.end, "range must not be empty");
    }

    #[test]
    fn hit_testing_round_trips_through_drawing() {
        let mut m = metrics();
        m.scroll = Vector::new(CELL_WIDTH * 3.0 + 10.0, CELL_HEIGHT * 7.0 + 4.0);
        for (row, col) in [(7u32, 3u32), (8, 4), (20, 9)] {
            let cell = CellRef::new(row, col);
            let rect = m.cell_rect(cell);
            let centre = Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
            assert_eq!(m.cell_at(centre), Some(cell), "{cell:?} round trip");
        }
        assert_eq!(m.cell_at(Point::new(HEADER_WIDTH - 1.0, 100.0)), None);
        assert_eq!(m.cell_at(Point::new(100.0, HEADER_HEIGHT - 1.0)), None);
    }

    #[test]
    fn the_fill_handle_sits_on_the_bottom_right_corner_of_the_selection() {
        let m = metrics();
        let selection = Bounds::new(CellRef::new(1, 1), CellRef::new(3, 2));
        let handle = m.fill_handle(selection);
        let corner = m.cell_rect(CellRef::new(3, 2));
        let expected = Point::new(corner.x + corner.width, corner.y + corner.height);
        let centre = Point::new(handle.x + handle.width / 2.0, handle.y + handle.height / 2.0);
        assert!((centre.x - expected.x).abs() < 0.01);
        assert!((centre.y - expected.y).abs() < 0.01);
        assert!(m.hits_fill_handle(selection, centre));
        let middle = m.cell_rect(CellRef::new(2, 1));
        assert!(!m.hits_fill_handle(selection, Point::new(middle.x + 10.0, middle.y + 10.0)));
    }

    #[test]
    fn scroll_is_clamped_to_the_sheet() {
        let m = metrics();
        let clamped = m.clamp_scroll(Vector::new(-50.0, -50.0));
        assert_eq!(clamped, Vector::new(0.0, 0.0));
        let clamped = m.clamp_scroll(Vector::new(f32::MAX, f32::MAX));
        assert!(clamped.x <= Dims::SPREADSHEET.cols as f32 * CELL_WIDTH);
        assert!(clamped.y <= Dims::SPREADSHEET.rows as f32 * CELL_HEIGHT);
    }

    #[test]
    fn scroll_to_show_brings_an_off_screen_cell_into_view() {
        let m = metrics();
        let start = Vector::new(0.0, 0.0);
        let scrolled = m.scroll_to_show(CellRef::new(200, 0), start);
        assert!(scrolled.y > 0.0);
        let rect = Metrics::new(scrolled, m.viewport, Dims::SPREADSHEET).cell_rect(CellRef::new(200, 0));
        assert!(rect.y >= HEADER_HEIGHT && rect.y + rect.height <= m.viewport.height);
        let scrolled = m.scroll_to_show(CellRef::new(5, 2), start);
        assert_eq!(scrolled, start);
    }

    #[test]
    fn scrollbars_only_appear_when_there_is_something_to_scroll() {
        let m = metrics();
        assert!(m.vertical_scrollbar().is_some());
        assert!(m.horizontal_scrollbar().is_some());

        let (track, thumb) = m.vertical_scrollbar().unwrap();
        assert!(thumb.height < track.height);
        assert!(thumb.y >= track.y);
    }

    #[test]
    fn numbers_are_rounded_to_fit_instead_of_truncated() {
        let rendered = fit_number(0.4333333333333333, 60.0, FONT_SIZE);
        assert!(rendered.starts_with("0.4"), "{rendered}");
        assert!(!rendered.contains('…'), "{rendered} should be rounded, not chopped");
        assert_eq!(fit_number(15.6, 100.0, FONT_SIZE), "15.6");
        assert_eq!(fit_number(1.0, 100.0, FONT_SIZE), "1");
    }

    #[test]
    fn text_is_truncated_to_fit_a_cell() {
        let short = "SUM";
        assert_eq!(fit_text(short, 100.0, FONT_SIZE), short);
        let long = "a very long piece of text that will not fit";
        let fitted = fit_text(long, 60.0, FONT_SIZE);
        assert!(fitted.ends_with('…'));
        assert!(fitted.chars().count() < long.chars().count());
    }

    #[test]
    fn both_axes_share_one_scrollbar_geometry() {
        let m = metrics();
        let (track, thumb) = m.vertical_scrollbar().unwrap();
        assert_eq!(m.scrollbar(Axis::Vertical), Some((track, thumb)));
        assert_eq!(m.horizontal_scrollbar(), m.scrollbar(Axis::Horizontal));
    }

    #[test]
    fn a_press_on_the_thumb_asks_for_a_drag() {
        let m = mid_scrolled();
        for axis in [Axis::Vertical, Axis::Horizontal] {
            let (_, thumb) = m.scrollbar(axis).unwrap();
            let centre = Point::new(thumb.x + thumb.width / 2.0, thumb.y + thumb.height / 2.0);
            assert_eq!(m.scrollbar_at(centre), Some(ScrollbarHit::Thumb(axis)), "{axis:?}");
        }
    }

    #[test]
    fn a_press_on_the_empty_track_pages_towards_the_press() {
        let m = mid_scrolled();
        let (track, thumb) = m.vertical_scrollbar().unwrap();
        let before = Point::new(track.x + SCROLLBAR / 2.0, track.y + 1.0);
        let after = Point::new(track.x + SCROLLBAR / 2.0, thumb.y + thumb.height + 2.0);
        assert_eq!(
            m.scrollbar_at(before),
            Some(ScrollbarHit::Track { axis: Axis::Vertical, forward: false })
        );
        assert_eq!(
            m.scrollbar_at(after),
            Some(ScrollbarHit::Track { axis: Axis::Vertical, forward: true })
        );

        let (track, thumb) = m.horizontal_scrollbar().unwrap();
        let before = Point::new(track.x + 1.0, track.y + SCROLLBAR / 2.0);
        let after = Point::new(thumb.x + thumb.width + 2.0, track.y + SCROLLBAR / 2.0);
        assert_eq!(
            m.scrollbar_at(before),
            Some(ScrollbarHit::Track { axis: Axis::Horizontal, forward: false })
        );
        assert_eq!(
            m.scrollbar_at(after),
            Some(ScrollbarHit::Track { axis: Axis::Horizontal, forward: true })
        );
    }

    #[test]
    fn the_corner_the_bars_share_belongs_to_the_one_drawn_last() {
        let m = mid_scrolled();
        let (vertical, _) = m.vertical_scrollbar().unwrap();
        let (horizontal, _) = m.horizontal_scrollbar().unwrap();
        let corner = Point::new(vertical.x + 1.0, horizontal.y + 1.0);
        match m.scrollbar_at(corner) {
            Some(ScrollbarHit::Thumb(Axis::Horizontal))
            | Some(ScrollbarHit::Track { axis: Axis::Horizontal, .. }) => {}
            other => panic!("expected the horizontal bar, got {other:?}"),
        }
    }

    #[test]
    fn the_grid_body_is_not_a_scrollbar() {
        let m = mid_scrolled();
        assert_eq!(m.scrollbar_at(Point::new(HEADER_WIDTH + 40.0, HEADER_HEIGHT + 40.0)), None);
        assert_eq!(m.scrollbar_at(Point::new(4.0, 4.0)), None);
    }

    #[test]
    fn a_drag_of_n_pixels_moves_the_thumb_exactly_n_pixels() {
        let m = mid_scrolled();
        for axis in [Axis::Vertical, Axis::Horizontal] {
            let delta = 37.0;
            let moved = axis.with(m.scroll, axis.of(m.scroll) + m.thumb_drag(axis, delta));
            assert!(m.clamp_scroll(moved) == moved, "{axis:?} should stay in range");

            let (_, before) = m.scrollbar(axis).unwrap();
            let (_, after) = Metrics::new(moved, m.viewport, Dims::SPREADSHEET).scrollbar(axis).unwrap();
            let travelled = axis.along(after.position()) - axis.along(before.position());
            assert!(
                (travelled - delta).abs() < 0.05,
                "{axis:?} thumb moved {travelled} px for a {delta} px drag"
            );
        }
    }

    #[test]
    fn a_full_thumb_travel_reaches_the_end_of_the_scroll() {
        let m = metrics();
        for axis in [Axis::Vertical, Axis::Horizontal] {
            let (track, thumb) = m.scrollbar(axis).unwrap();
            let span = axis.length(track.size());
            let travel = span - axis.length(thumb.size());
            let end = m.thumb_drag(axis, travel);
            let extent = Scrolling::extent(m.content_extent(axis), span);
            assert!((end - extent).abs() <= extent * 1e-4, "{axis:?} ended at {end}, not {extent}");
        }
    }

    #[test]
    fn a_track_press_pages_by_about_one_viewport() {
        let m = metrics();
        let grid = m.grid_size();
        assert_eq!(m.page_scroll(Axis::Vertical, true), grid.height);
        assert_eq!(m.page_scroll(Axis::Vertical, false), -grid.height);
        assert_eq!(m.page_scroll(Axis::Horizontal, true), grid.width);
        assert_eq!(m.page_scroll(Axis::Horizontal, false), -grid.width);
    }

    #[test]
    fn dragging_past_the_end_overscrolls_nothing() {
        let m = metrics();
        let far = m.thumb_drag(Axis::Vertical, 10_000.0);
        let clamped = m.clamp_scroll(Vector::new(0.0, far));
        assert!(clamped.y > 0.0);
        assert!(clamped.y <= m.content_extent(Axis::Vertical));
    }

    // A model that owes nothing to any formula engine. If the grid can be
    // driven by this, it can be driven by anyone's data.
    struct Fixture;

    impl SheetModel for Fixture {
        fn dims(&self) -> Dims {
            Dims { rows: 5, cols: 3 }
        }

        fn value(&self, cell: CellRef) -> CellValue {
            match (cell.row, cell.col) {
                (0, 0) => CellValue::Number(41.5),
                (1, 1) => CellValue::Text("a note".into()),
                (2, 2) => CellValue::Error("#DIV/0!".into()),
                _ => CellValue::Empty,
            }
        }
    }

    #[test]
    fn a_foreign_model_decides_how_big_the_sheet_is() {
        let model = Fixture;
        let m = Metrics::new(Vector::new(0.0, 0.0), Size::new(1000.0, 700.0), model.dims());

        // Five rows, however tall the viewport: no scrolling past the end.
        assert_eq!(m.visible_rows(), 0..5, "a five-row sheet ends at row five");
        assert_eq!(m.visible_cols(), 0..3);

        // A sheet that already fits has no scrollbars to offer.
        assert!(m.vertical_scrollbar().is_none());
        assert!(m.horizontal_scrollbar().is_none());

        // And nothing outside it is reachable, by click or by drag.
        let past_the_end = Point::new(HEADER_WIDTH + 10.0, HEADER_HEIGHT + 6.0 * CELL_HEIGHT);
        assert_eq!(m.cell_at(past_the_end), None);
        assert_eq!(m.clamp_scroll(Vector::new(f32::MAX, f32::MAX)), Vector::new(0.0, 0.0));
        assert!(m.cell_at(Point::new(HEADER_WIDTH + 1.0, HEADER_HEIGHT + 1.0)).is_some());
    }

    #[test]
    fn the_grid_paints_whatever_the_model_reports() {
        let model = Fixture;
        assert_eq!(model.dims().rows, 5);
        assert_eq!(model.value(CellRef::new(0, 0)).as_text(), "41.5");
        assert_eq!(model.value(CellRef::new(0, 0)).alignment(), alignment::Horizontal::Right);
        assert_eq!(model.value(CellRef::new(1, 1)).alignment(), alignment::Horizontal::Left);
        assert!(model.value(CellRef::new(2, 2)).is_error());
        assert!(model.value(CellRef::new(9, 9)).is_empty());
    }

    // The grid hands events back through a `fn` pointer, which can't capture
    // anything, so the test collects them in a thread-local instead.
    thread_local! {
        static PORT: RefCell<Vec<GridEvent>> = RefCell::new(Vec::new());
    }

    fn record(event: GridEvent) {
        PORT.with(|port| port.borrow_mut().push(event));
    }

    #[test]
    fn viewport_reports_the_canvas_bounds_not_the_window_size() {
        // The window is bigger than the grid on both axes: the bars own the top
        // and bottom and the canvas is the strip between them, offset down.
        let window = Size::new(1600.0, 1000.0);
        let canvas = Rectangle::new(Point::new(0.0, 96.0), Size::new(1200.0, 700.0));
        assert_ne!(window, canvas.size(), "the two must differ or the test proves nothing");

        let program = GridProgram {
            model: Fixture,
            selection: Bounds::single(CellRef::new(0, 0)),
            active: CellRef::new(0, 0),
            fill_preview: None,
            scroll: Vector::new(0.0, 0.0),
            active_scrollbar: None,
            palette: GardenPalette::light(),
            on_event: record,
        };

        PORT.with(|port| port.borrow_mut().clear());
        let resized = canvas::Event::Window(window::Event::Resized(window));
        let action = canvas::Program::update(
            &program,
            &mut (),
            &resized,
            canvas,
            mouse::Cursor::Unavailable,
        );
        assert!(action.is_some(), "a resize should reach the host");

        PORT.with(|port| {
            let port = port.borrow();
            assert_eq!(port.len(), 1, "one resize, one event");
            assert_eq!(port[0], GridEvent::Viewport(canvas.size()));
            assert_ne!(port[0], GridEvent::Viewport(window), "the window size is the old bug");
        });
    }
}
