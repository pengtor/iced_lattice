use iced::mouse;
use iced::widget::canvas::{self, Geometry, LineCap, Path, Stroke, Text};
use iced::widget::canvas::{Action, Frame};
use iced::{window, Font, Pixels, Point, Rectangle, Renderer, Size, Theme, Vector};
use iced::alignment;

use engine::{CellRef, Bounds, Sheet, Value, MAX_COLS, MAX_ROWS};

use crate::state::Message;
use crate::theme::{self, GardenPalette};

pub const HEADER_WIDTH: f32 = 54.0;
pub const HEADER_HEIGHT: f32 = 24.0;
pub const CELL_WIDTH: f32 = 104.0;
pub const CELL_HEIGHT: f32 = 26.0;
pub const BUFFER: u32 = 1;
pub const FILL_HANDLE: f32 = 7.0;
pub const SCROLLBAR: f32 = 12.0;

const FONT_SIZE: f32 = 13.0;
const TEXT_PADDING: f32 = 7.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    // Along the track: y for vertical, x for horizontal
    pub fn along(self, point: Point) -> f32 {
        match self {
            Axis::Vertical => point.y,
            Axis::Horizontal => point.x,
        }
    }

    pub fn of(self, scroll: Vector) -> f32 {
        match self {
            Axis::Vertical => scroll.y,
            Axis::Horizontal => scroll.x,
        }
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollbarHit {
    Thumb(Axis),
    // Past the thumb pages forward, before it pages back
    Track { axis: Axis, forward: bool },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub scroll: Vector,
    pub viewport: Size,
}

impl Metrics {
    pub fn new(scroll: Vector, viewport: Size) -> Self {
        Metrics { scroll, viewport }
    }

    pub fn grid_size(&self) -> Size {
        Size::new(
            (self.viewport.width - HEADER_WIDTH).max(0.0),
            (self.viewport.height - HEADER_HEIGHT).max(0.0),
        )
    }

    pub fn visible_rows(&self) -> std::ops::Range<u32> {
        let first = (self.scroll.y.max(0.0) / CELL_HEIGHT).floor() as u32;
        let first = first.saturating_sub(BUFFER);
        let span = (self.grid_size().height / CELL_HEIGHT).ceil() as u32 + 1 + 2 * BUFFER;
        let last = first.saturating_add(span).min(MAX_ROWS);
        first..last.max(first + 1).min(MAX_ROWS)
    }

    pub fn visible_cols(&self) -> std::ops::Range<u32> {
        let first = (self.scroll.x.max(0.0) / CELL_WIDTH).floor() as u32;
        let first = first.saturating_sub(BUFFER);
        let span = (self.grid_size().width / CELL_WIDTH).ceil() as u32 + 1 + 2 * BUFFER;
        let last = first.saturating_add(span).min(MAX_COLS);
        first..last.max(first + 1).min(MAX_COLS)
    }

    pub fn cell_rect(&self, cell: CellRef) -> Rectangle {
        Rectangle::new(
            Point::new(
                HEADER_WIDTH + cell.col as f32 * CELL_WIDTH - self.scroll.x,
                HEADER_HEIGHT + cell.row as f32 * CELL_HEIGHT - self.scroll.y,
            ),
            Size::new(CELL_WIDTH, CELL_HEIGHT),
        )
    }

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

    pub fn row_header_rect(&self, row: u32) -> Rectangle {
        Rectangle::new(
            Point::new(0.0, HEADER_HEIGHT + row as f32 * CELL_HEIGHT - self.scroll.y),
            Size::new(HEADER_WIDTH, CELL_HEIGHT),
        )
    }

    pub fn col_header_rect(&self, col: u32) -> Rectangle {
        Rectangle::new(
            Point::new(HEADER_WIDTH + col as f32 * CELL_WIDTH - self.scroll.x, 0.0),
            Size::new(CELL_WIDTH, HEADER_HEIGHT),
        )
    }

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

    pub fn clamp_scroll(&self, scroll: Vector) -> Vector {
        Vector::new(
            scroll.x.clamp(0.0, Scrolling::extent(MAX_COLS as f32 * CELL_WIDTH, self.grid_size().width)),
            scroll.y.clamp(0.0, Scrolling::extent(MAX_ROWS as f32 * CELL_HEIGHT, self.grid_size().height)),
        )
    }

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

    pub fn scrollbar(&self, axis: Axis) -> Option<(Rectangle, Rectangle)> {
        let (track, content, offset) = match axis {
            Axis::Vertical => (
                Rectangle::new(
                    Point::new(self.viewport.width - SCROLLBAR, HEADER_HEIGHT),
                    Size::new(SCROLLBAR, (self.viewport.height - HEADER_HEIGHT).max(0.0)),
                ),
                MAX_ROWS as f32 * CELL_HEIGHT,
                self.scroll.y,
            ),
            Axis::Horizontal => (
                Rectangle::new(
                    Point::new(HEADER_WIDTH, self.viewport.height - SCROLLBAR),
                    Size::new((self.viewport.width - HEADER_WIDTH).max(0.0), SCROLLBAR),
                ),
                MAX_COLS as f32 * CELL_WIDTH,
                self.scroll.x,
            ),
        };
        Scrolling::thumb(track, content, offset)
    }

    pub fn vertical_scrollbar(&self) -> Option<(Rectangle, Rectangle)> {
        self.scrollbar(Axis::Vertical)
    }

    pub fn horizontal_scrollbar(&self) -> Option<(Rectangle, Rectangle)> {
        self.scrollbar(Axis::Horizontal)
    }

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

    // Dragged pixels -> scroll offset, at the drawn thumb's ratio
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

    // Empty track pages by about one viewport
    pub fn page_scroll(&self, axis: Axis, forward: bool) -> f32 {
        let page = axis.length(self.grid_size());
        if forward { page } else { -page }
    }

    fn content_extent(&self, axis: Axis) -> f32 {
        match axis {
            Axis::Vertical => MAX_ROWS as f32 * CELL_HEIGHT,
            Axis::Horizontal => MAX_COLS as f32 * CELL_WIDTH,
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

// Canvas text isn't clipped; fit it by average advance width
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

// A number that doesn't fit is rounded, not truncated
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

fn horizontal_alignment(value: &Value) -> alignment::Horizontal {
    match value {
        Value::Number(_) => alignment::Horizontal::Right,
        _ => alignment::Horizontal::Left,
    }
}

pub struct GridProgram<'a> {
    pub sheet: &'a Sheet,
    pub selection: Bounds,
    pub active: CellRef,
    pub fill_preview: Option<Bounds>,
    pub scroll: Vector,
    // The bar being dragged: it paints solid while held
    pub active_scrollbar: Option<Axis>,
    // Passed in explicitly: iced Theme carries only six colours
    pub palette: GardenPalette,
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
                // Cursor positions are window-absolute; canvas works in local coords
                let position = cursor.position_in(bounds)?;
                Some(Action::publish(Message::PointerPressed { position, viewport: bounds.size() }).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                Some(Action::publish(Message::PointerReleased).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                // Deliberately unclipped so drags leaving the grid keep extending
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
            // Resize arrives before pointer events; keeps scroll clamping honest
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
        let hairline = Stroke::default().with_width(theme::HAIRLINE).with_color(self.palette.lattice);

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
                Stroke::default().with_width(theme::FOCUS_BORDER).with_color(self.palette.leaf),
            );
        }
        frame.stroke_rectangle(
            metrics.bounds_rect(self.selection).position(),
            metrics.bounds_rect(self.selection).size(),
            Stroke::default().with_width(theme::FOCUS_BORDER).with_color(self.palette.leaf),
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

impl GridProgram<'_> {
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
            let label = engine::addr::col_name(col);
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
            let outline = Stroke::default().with_width(theme::FOCUS_BORDER).with_color(self.palette.leaf);
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

    // Half way along both tracks: room on both sides
    fn mid_scrolled() -> Metrics {
        let m = metrics();
        let grid = m.grid_size();
        let half = Vector::new(
            Scrolling::extent(MAX_COLS as f32 * CELL_WIDTH, grid.width) / 2.0,
            Scrolling::extent(MAX_ROWS as f32 * CELL_HEIGHT, grid.height) / 2.0,
        );
        Metrics::new(m.clamp_scroll(half), m.viewport)
    }

    #[test]
    fn visible_ranges_cover_the_viewport_without_touching_the_sheet_size() {
        let m = metrics();
        let rows = m.visible_rows();
        let cols = m.visible_cols();
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
        assert!(clamped.x <= MAX_COLS as f32 * CELL_WIDTH);
        assert!(clamped.y <= MAX_ROWS as f32 * CELL_HEIGHT);
    }

    #[test]
    fn scroll_to_show_brings_an_off_screen_cell_into_view() {
        let m = metrics();
        let start = Vector::new(0.0, 0.0);
        let scrolled = m.scroll_to_show(CellRef::new(200, 0), start);
        assert!(scrolled.y > 0.0);
        let rect = Metrics::new(scrolled, m.viewport).cell_rect(CellRef::new(200, 0));
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
            let (_, after) = Metrics::new(moved, m.viewport).scrollbar(axis).unwrap();
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

    #[test]
    fn numbers_are_right_aligned_and_text_is_left_aligned() {
        assert_eq!(horizontal_alignment(&Value::Number(1.0)), alignment::Horizontal::Right);
        assert_eq!(horizontal_alignment(&Value::Text("x".into())), alignment::Horizontal::Left);
        assert_eq!(horizontal_alignment(&Value::Bool(true)), alignment::Horizontal::Left);
    }
}
