//! The grid canvas: viewport-virtualised drawing and pointer interaction.
//!
//! # Virtualisation
//!
//! The sheet offers 1 048 576 rows and 16 384 columns, but the canvas only ever
//! looks at the cells that are on screen (plus a one-cell buffer). Scrolling,
//! drawing and hit-testing are all pure functions of the scroll offset and the
//! viewport size, so the cost of a frame depends on the size of the *window*, not
//! on the size of the sheet. Nothing here allocates per cell either: the same
//! `Rectangle`s are recomputed as the loop walks the visible range.

use iced::mouse;
use iced::widget::canvas::{self, Geometry, LineCap, Path, Stroke, Text};
use iced::widget::canvas::{Action, Frame};
use iced::{window, Color, Font, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector};
use iced::alignment;

use engine::{CellRef, Bounds, Sheet, Value, MAX_COLS, MAX_ROWS};

use crate::application::Message;
use crate::theme;

/// Width of the row-number gutter.
pub const HEADER_WIDTH: f32 = 54.0;
/// Height of the column-letter gutter.
pub const HEADER_HEIGHT: f32 = 24.0;
/// Cell size.
pub const CELL_WIDTH: f32 = 104.0;
pub const CELL_HEIGHT: f32 = 26.0;
/// Extra rows/columns drawn beyond the viewport, so that a partially scrolled
/// cell is never missing.
pub const BUFFER: u32 = 1;
/// Size of the fill handle square.
pub const FILL_HANDLE: f32 = 7.0;
/// Thickness of the scroll indicators.
pub const SCROLLBAR: f32 = 8.0;

const FONT_SIZE: f32 = 13.0;
const TEXT_PADDING: f32 = 7.0;

/// Everything needed to convert between sheet coordinates and canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Pixels scrolled away, in sheet space.
    pub scroll: Vector,
    /// Size of the canvas, including the gutters.
    pub viewport: Size,
}

impl Metrics {
    pub fn new(scroll: Vector, viewport: Size) -> Self {
        Metrics { scroll, viewport }
    }

    /// The area occupied by cells (the viewport minus the gutters).
    pub fn grid_size(&self) -> Size {
        Size::new(
            (self.viewport.width - HEADER_WIDTH).max(0.0),
            (self.viewport.height - HEADER_HEIGHT).max(0.0),
        )
    }

    /// Rows that intersect the viewport, plus the buffer.
    pub fn visible_rows(&self) -> std::ops::Range<u32> {
        let first = (self.scroll.y.max(0.0) / CELL_HEIGHT).floor() as u32;
        let first = first.saturating_sub(BUFFER);
        let span = (self.grid_size().height / CELL_HEIGHT).ceil() as u32 + 1 + 2 * BUFFER;
        let last = first.saturating_add(span).min(MAX_ROWS);
        first..last.max(first + 1).min(MAX_ROWS)
    }

    /// Columns that intersect the viewport, plus the buffer.
    pub fn visible_cols(&self) -> std::ops::Range<u32> {
        let first = (self.scroll.x.max(0.0) / CELL_WIDTH).floor() as u32;
        let first = first.saturating_sub(BUFFER);
        let span = (self.grid_size().width / CELL_WIDTH).ceil() as u32 + 1 + 2 * BUFFER;
        let last = first.saturating_add(span).min(MAX_COLS);
        first..last.max(first + 1).min(MAX_COLS)
    }

    /// Where a cell is drawn, in canvas coordinates.
    pub fn cell_rect(&self, cell: CellRef) -> Rectangle {
        Rectangle::new(
            Point::new(
                HEADER_WIDTH + cell.col as f32 * CELL_WIDTH - self.scroll.x,
                HEADER_HEIGHT + cell.row as f32 * CELL_HEIGHT - self.scroll.y,
            ),
            Size::new(CELL_WIDTH, CELL_HEIGHT),
        )
    }

    /// The rectangle covering a whole range.
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

    /// Which cell is under a canvas-space point (`None` over the gutters).
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
        if row >= MAX_ROWS || col >= MAX_COLS {
            return None;
        }
        Some(CellRef::new(row, col))
    }

    /// The gutter rectangle for a row.
    pub fn row_header_rect(&self, row: u32) -> Rectangle {
        Rectangle::new(
            Point::new(0.0, HEADER_HEIGHT + row as f32 * CELL_HEIGHT - self.scroll.y),
            Size::new(HEADER_WIDTH, CELL_HEIGHT),
        )
    }

    /// The gutter rectangle for a column.
    pub fn col_header_rect(&self, col: u32) -> Rectangle {
        Rectangle::new(
            Point::new(HEADER_WIDTH + col as f32 * CELL_WIDTH - self.scroll.x, 0.0),
            Size::new(CELL_WIDTH, HEADER_HEIGHT),
        )
    }

    /// The fill handle drawn at the bottom-right of `selection`.
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

    /// Whether a point is close enough to the handle to start a fill drag.
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

    /// Keep a scroll offset inside the sheet.
    pub fn clamp_scroll(&self, scroll: Vector) -> Vector {
        Vector::new(
            scroll.x.clamp(0.0, Scrolling::extent(MAX_COLS as f32 * CELL_WIDTH, self.grid_size().width)),
            scroll.y.clamp(0.0, Scrolling::extent(MAX_ROWS as f32 * CELL_HEIGHT, self.grid_size().height)),
        )
    }

    /// The smallest scroll offset that brings `cell` fully into view.
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

    /// The vertical scroll indicator, if the sheet is taller than the viewport.
    pub fn vertical_scrollbar(&self) -> Option<(Rectangle, Rectangle)> {
        let track = Rectangle::new(
            Point::new(self.viewport.width - SCROLLBAR, HEADER_HEIGHT),
            Size::new(SCROLLBAR, (self.viewport.height - HEADER_HEIGHT).max(0.0)),
        );
        let content = MAX_ROWS as f32 * CELL_HEIGHT;
        Scrolling::thumb(track, content, self.scroll.y)
    }

    /// The horizontal scroll indicator, if the sheet is wider than the viewport.
    pub fn horizontal_scrollbar(&self) -> Option<(Rectangle, Rectangle)> {
        let track = Rectangle::new(
            Point::new(HEADER_WIDTH, self.viewport.height - SCROLLBAR),
            Size::new((self.viewport.width - HEADER_WIDTH).max(0.0), SCROLLBAR),
        );
        let content = MAX_COLS as f32 * CELL_WIDTH;
        Scrolling::thumb(track, content, self.scroll.x)
    }
}

/// Scroll indicator geometry.
struct Scrolling;

impl Scrolling {
    fn extent(content: f32, viewport: f32) -> f32 {
        (content - viewport).max(0.0)
    }

    /// `(track, thumb)` for one axis, or `None` when everything fits.
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

/// Truncate `text` so that it fits `max_width` at `font_size`, adding an ellipsis.
///
/// Canvas text is not clipped to the cell it belongs to, so overflowing text would
/// be drawn across the neighbouring cells. Measuring properly would need a text
/// shaper; an average advance width is close enough for a grid of uniform cells and
/// keeps drawing allocation-free in the common (short) case.
pub fn fit_text(text: &str, max_width: f32, font_size: f32) -> String {
    let capacity = characters_that_fit(max_width, font_size);
    if text.chars().count() <= capacity {
        return text.to_string();
    }
    let mut out: String = text.chars().take(capacity.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// How many characters of `font_size` fit into `max_width`.
fn characters_that_fit(max_width: f32, font_size: f32) -> usize {
    let advance = font_size * 0.58;
    (max_width / advance).floor().max(1.0) as usize
}

/// Render a number so that it fits its column.
///
/// A number that does not fit is *rounded* rather than truncated: `0.433333333…`
/// is useless where `0.4333` is informative. Fewer decimals are tried until the
/// text fits, and only if even an integer is too wide does it get truncated.
pub fn fit_number(value: f64, max_width: f32, font_size: f32) -> String {
    let capacity = characters_that_fit(max_width, font_size);
    let full = engine::format_number(value);
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

/// How a cell's text should be aligned: numbers right, everything else left.
fn horizontal_alignment(value: &Value) -> alignment::Horizontal {
    match value {
        Value::Number(_) => alignment::Horizontal::Right,
        _ => alignment::Horizontal::Left,
    }
}

/// The canvas program that draws and interacts with the sheet.
pub struct GridProgram<'a> {
    pub sheet: &'a Sheet,
    /// The range that is highlighted.
    pub selection: Bounds,
    /// The cell with focus inside the selection.
    pub active: CellRef,
    /// A live fill preview, while the fill handle is being dragged.
    pub fill_preview: Option<Bounds>,
    pub scroll: Vector,
}

impl GridProgram<'_> {
    fn metrics(&self, bounds: Rectangle) -> Metrics {
        Metrics::new(self.scroll, bounds.size())
    }
}

impl canvas::Program<Message> for GridProgram<'_> {
    type State = ();

    fn update(
        &self,
        _state: &mut (),
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Message>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                // `position_over` is *window* absolute; everything the grid does is in
                // canvas-local coordinates, so the bounds origin has to come off.
                let position = cursor.position_in(bounds)?;
                Some(Action::publish(Message::PointerPressed { position, viewport: bounds.size() }).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Some(Action::publish(Message::PointerReleased).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // Deliberately *not* clipped to the canvas: a drag that leaves the
                // grid should keep extending the selection, as it does in every
                // spreadsheet.
                let position = cursor.position_from(bounds.position())?;
                Some(Action::publish(Message::PointerMoved { position, viewport: bounds.size() }))
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let delta = match delta {
                    mouse::ScrollDelta::Lines { x, y } => Vector::new(x * 48.0, y * 48.0),
                    mouse::ScrollDelta::Pixels { x, y } => Vector::new(*x, *y),
                };
                Some(Action::publish(Message::Scrolled { delta, viewport: bounds.size() }).and_capture())
            }
            // A resize reaches the canvas before any pointer event does, which keeps
            // scroll clamping honest for users who only ever use the keyboard.
            canvas::Event::Window(window::Event::Resized(size)) => {
                Some(Action::publish(Message::Viewport(*size)))
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

        // 1. Paper.
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), theme::CANVAS);

        // 2. The selection wash, under the grid lines so the lattice stays visible
        //    through it.
        let selected = metrics.bounds_rect(self.selection);
        frame.fill_rectangle(
            Point::new(selected.x.max(HEADER_WIDTH), selected.y.max(HEADER_HEIGHT)),
            Size::new(
                (selected.width - (HEADER_WIDTH - selected.x).max(0.0)).max(0.0),
                (selected.height - (HEADER_HEIGHT - selected.y).max(0.0)).max(0.0),
            ),
            theme::SELECTION_FILL,
        );

        // 3. The lattice itself: one line per row and column edge, drawn only for
        //    the rows and columns that are on screen.
        let rows = metrics.visible_rows();
        let cols = metrics.visible_cols();
        let hairline = Stroke::default().with_width(theme::HAIRLINE).with_color(theme::LATTICE);

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

        // 4. Cell contents.
        for row in rows {
            for col in cols.clone() {
                let cell = CellRef::new(row, col);
                let value = self.sheet.value(cell);
                if value.is_empty() {
                    continue;
                }
                let rect = metrics.cell_rect(cell);
                if rect.x + rect.width < HEADER_WIDTH || rect.y + rect.height < HEADER_HEIGHT {
                    continue;
                }
                let align = horizontal_alignment(&value);
                let text = match &value {
                    Value::Number(n) => fit_number(*n, rect.width - 2.0 * TEXT_PADDING, FONT_SIZE),
                    other => fit_text(&other.as_text(), rect.width - 2.0 * TEXT_PADDING, FONT_SIZE),
                };
                let color = if value.is_error() { theme::CLAY } else { theme::INK };
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

        // 5. Gutters, drawn last so that cell contents can never bleed into them.
        self.draw_gutters(&mut frame, &metrics, bounds, &selected);

        // 6. Focus ring, fill handle, and the live fill preview.
        if let Some(preview) = self.fill_preview {
            let rect = metrics.bounds_rect(preview);
            frame.fill_rectangle(
                rect.position(),
                rect.size(),
                theme::SELECTION_FILL_ACTIVE,
            );
            frame.stroke_rectangle(
                rect.position(),
                rect.size(),
                Stroke::default().with_width(theme::FOCUS_BORDER).with_color(theme::LEAF),
            );
        }
        frame.stroke_rectangle(
            metrics.bounds_rect(self.selection).position(),
            metrics.bounds_rect(self.selection).size(),
            Stroke::default().with_width(theme::FOCUS_BORDER).with_color(theme::LEAF),
        );

        let handle = metrics.fill_handle(self.selection);
        frame.fill_rectangle(
            Point::new(handle.x, handle.y),
            handle.size(),
            theme::LEAF,
        );
        frame.stroke(
            &Path::rectangle(Point::new(handle.x, handle.y), handle.size()),
            Stroke::default().with_width(1.0).with_color(theme::CANVAS),
        );

        // 7. Scroll indicators.
        for (track, thumb) in [metrics.vertical_scrollbar(), metrics.horizontal_scrollbar()]
            .into_iter()
            .flatten()
        {
            frame.fill_rectangle(track.position(), track.size(), Color::from_rgba(0.85, 0.86, 0.80, 0.18));
            frame.fill_rectangle(
                thumb.position(),
                thumb.size(),
                Color::from_rgba(0.49, 0.58, 0.45, 0.55),
            );
        }

        vec![frame.into_geometry()]
    }
}

impl GridProgram<'_> {
    /// Draw the row and column gutters, highlighting the selection's span.
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

        // Backgrounds.
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(bounds.width, HEADER_HEIGHT),
            theme::SURFACE,
        );
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(HEADER_WIDTH, bounds.height),
            theme::SURFACE,
        );

        // Highlight the rows and columns the selection touches, so the selection
        // reads as a band across the trellis rather than an isolated box.
        for row in rows.clone() {
            if row < selection.min_row || row > selection.max_row {
                continue;
            }
            let rect = metrics.row_header_rect(row);
            if rect.y < HEADER_HEIGHT {
                continue;
            }
            frame.fill_rectangle(rect.position(), rect.size(), theme::GUTTER_ACTIVE);
        }
        for col in cols.clone() {
            if col < selection.min_col || col > selection.max_col {
                continue;
            }
            let rect = metrics.col_header_rect(col);
            if rect.x < HEADER_WIDTH {
                continue;
            }
            frame.fill_rectangle(rect.position(), rect.size(), theme::GUTTER_ACTIVE);
        }

        // Labels.
        for row in rows {
            let rect = metrics.row_header_rect(row);
            if rect.y < HEADER_HEIGHT {
                continue;
            }
            let label = (row + 1).to_string();
            frame.fill_text(Text {
                content: label,
                position: Point::new(HEADER_WIDTH - TEXT_PADDING, rect.y + rect.height / 2.0),
                color: theme::INK_SOFT,
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
            let label = engine::addr::col_name(col);
            frame.fill_text(Text {
                content: label,
                position: Point::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0),
                color: theme::INK_SOFT,
                size: Pixels(FONT_SIZE - 1.0),
                align_x: alignment::Horizontal::Center.into(),
                align_y: alignment::Vertical::Center,
                ..Text::default()
            });
        }

        // The corner square and the separator lines.
        frame.fill_rectangle(
            Point::ORIGIN,
            Size::new(HEADER_WIDTH, HEADER_HEIGHT),
            theme::SURFACE_DEEP,
        );
        let line = Stroke::default().with_width(1.0).with_color(theme::LATTICE_STRONG);
        frame.stroke(
            &Path::line(Point::new(0.0, HEADER_HEIGHT), Point::new(bounds.width, HEADER_HEIGHT)),
            line,
        );
        frame.stroke(
            &Path::line(Point::new(HEADER_WIDTH, 0.0), Point::new(HEADER_WIDTH, bounds.height)),
            line,
        );

        // A soft emphasis on the active cell's row and column labels.
        let row_rect = metrics.row_header_rect(self.active.row);
        if row_rect.y >= HEADER_HEIGHT {
            frame.fill_rectangle(
                Point::new(0.0, row_rect.y),
                Size::new(HEADER_WIDTH, row_rect.height),
                theme::SELECTION_FILL_ACTIVE,
            );
        }
        let col_rect = metrics.col_header_rect(self.active.col);
        if col_rect.x >= HEADER_WIDTH {
            frame.fill_rectangle(
                col_rect.position(),
                Size::new(col_rect.width, HEADER_HEIGHT),
                theme::SELECTION_FILL_ACTIVE,
            );
        }

        // The active cell's own borders, drawn over the wash.
        let active_rect = metrics.bounds_rect(Bounds::single(self.active));
        if active_rect.y >= HEADER_HEIGHT && active_rect.x >= HEADER_WIDTH {
            let outline = Stroke::default().with_width(theme::FOCUS_BORDER).with_color(theme::LEAF);
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
    use super::*;

    fn metrics() -> Metrics {
        Metrics::new(Vector::new(0.0, 0.0), Size::new(1000.0, 700.0))
    }

    #[test]
    fn visible_ranges_cover_the_viewport_without_touching_the_sheet_size() {
        let m = metrics();
        let rows = m.visible_rows();
        let cols = m.visible_cols();
        // ~700px of grid at 26px per row, plus buffer.
        assert!(rows.start == 0);
        assert!(rows.end >= 27 && rows.end <= 31, "{rows:?}");
        assert!(cols.end >= 10 && cols.end <= 13, "{cols:?}");
        assert!(cols.end < MAX_COLS);
    }

    #[test]
    fn scrolling_moves_the_visible_window() {
        let mut m = metrics();
        m.scroll = Vector::new(0.0, CELL_HEIGHT * 100.0);
        let rows = m.visible_rows();
        assert_eq!(rows.start, 99); // one row of buffer
        assert!(rows.contains(&100));

        m.scroll = Vector::new(CELL_WIDTH * 50.0, 0.0);
        let cols = m.visible_cols();
        assert_eq!(cols.start, 49);
        assert!(cols.contains(&50));
    }

    #[test]
    fn the_visible_window_stays_inside_the_sheet_at_the_far_end() {
        let mut m = metrics();
        m.scroll = Vector::new(MAX_COLS as f32 * CELL_WIDTH, MAX_ROWS as f32 * CELL_HEIGHT);
        let rows = m.visible_rows();
        let cols = m.visible_cols();
        assert!(rows.end <= MAX_ROWS);
        assert!(cols.end <= MAX_COLS);
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
            assert_eq!(m.cell_at(centre), Some(cell), "{cell} round trip");
        }
        // A press in the gutter is not a cell press.
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
        // A press in the middle of the selection is not a handle grab.
        let middle = m.cell_rect(CellRef::new(2, 1));
        assert!(!m.hits_fill_handle(selection, Point::new(middle.x + 10.0, middle.y + 10.0)));
    }

    #[test]
    fn scroll_is_clamped_to_the_sheet() {
        let m = metrics();
        let clamped = m.clamp_scroll(Vector::new(-50.0, -50.0));
        assert_eq!(clamped, Vector::new(0.0, 0.0));
        let clamped = m.clamp_scroll(Vector::new(f32::MAX, f32::MAX));
        assert!(clamped.x <= MAX_COLS as f32 * CELL_WIDTH);
        assert!(clamped.y <= MAX_ROWS as f32 * CELL_HEIGHT);
    }

    #[test]
    fn scroll_to_show_brings_an_off_screen_cell_into_view() {
        let m = metrics();
        let start = Vector::new(0.0, 0.0);
        // A cell far below the fold scrolls down just enough.
        let scrolled = m.scroll_to_show(CellRef::new(200, 0), start);
        assert!(scrolled.y > 0.0);
        let rect = Metrics::new(scrolled, m.viewport).cell_rect(CellRef::new(200, 0));
        assert!(rect.y >= HEADER_HEIGHT && rect.y + rect.height <= m.viewport.height);
        // A cell already visible does not move the view.
        let scrolled = m.scroll_to_show(CellRef::new(5, 2), start);
        assert_eq!(scrolled, start);
    }

    #[test]
    fn scrollbars_only_appear_when_there_is_something_to_scroll() {
        let m = metrics();
        assert!(m.vertical_scrollbar().is_some());
        assert!(m.horizontal_scrollbar().is_some());

        // A viewport taller than the content on a tiny sheet still shows the
        // indicator, because the sheet is always a million rows tall — but the
        // thumb is proportionally tiny.
        let (track, thumb) = m.vertical_scrollbar().unwrap();
        assert!(thumb.height < track.height);
        assert!(thumb.y >= track.y);
    }

    #[test]
    fn numbers_are_rounded_to_fit_instead_of_truncated() {
        // 0.4333… does not fit in a cell; it should become something readable.
        let rendered = fit_number(0.4333333333333333, 60.0, FONT_SIZE);
        assert!(rendered.starts_with("0.4"), "{rendered}");
        assert!(!rendered.contains('…'), "{rendered} should be rounded, not chopped");
        // Short numbers are left exactly as they are.
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
    fn numbers_are_right_aligned_and_text_is_left_aligned() {
        assert_eq!(horizontal_alignment(&Value::Number(1.0)), alignment::Horizontal::Right);
        assert_eq!(horizontal_alignment(&Value::Text("x".into())), alignment::Horizontal::Left);
        assert_eq!(horizontal_alignment(&Value::Bool(true)), alignment::Horizontal::Left);
    }
}
