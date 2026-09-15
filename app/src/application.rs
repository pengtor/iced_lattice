//! Application state and the Elm-style update loop.
//!
//! The engine does the work; this module owns the *interaction*: what is selected,
//! what is being edited, where the viewport is, and how pointer and keyboard events
//! turn into engine calls. Everything that can be decided without a window is a
//! plain method on [`Lattice`], which is what makes it testable.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::advanced::widget::operation::focusable;
use iced::advanced::widget::operate;
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::widget::canvas;
use iced::widget::{button, column, container, row, stack, text, text_input, Space};
use iced::{
    alignment, window, Element, Font, Length, Padding, Point, Size, Subscription, Task, Vector,
};

use engine::sheet::Input;
use engine::{Bounds, CellRef, Sheet, Value, MAX_COLS, MAX_ROWS};

use crate::grid::{GridProgram, Metrics, CELL_HEIGHT, CELL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH};
use crate::theme;

/// Widget ids, so focus can be moved around.
pub const FORMULA_BAR: &str = "lattice-formula-bar";
pub const CELL_EDITOR: &str = "lattice-cell-editor";

/// Two presses on the same cell within this window start an edit.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// A selection larger than this is not summarised in the status bar.
const SUMMARY_LIMIT: u64 = 50_000;

/// A rectangular selection plus the single active cell inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The corner that stays put while the selection is extended.
    pub anchor: CellRef,
    /// The cell that has focus.
    pub active: CellRef,
}

impl Selection {
    pub fn single(cell: CellRef) -> Selection {
        Selection { anchor: cell, active: cell }
    }

    pub fn bounds(&self) -> Bounds {
        Bounds::new(self.anchor, self.active)
    }

    pub fn is_single(&self) -> bool {
        self.anchor == self.active
    }
}

/// What the pointer is currently doing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    /// Dragging out a range from the cell that was pressed.
    Selecting,
    /// Dragging the fill handle; holds the range being previewed.
    Filling(Bounds),
}

/// An in-progress edit.
#[derive(Clone, Debug, PartialEq)]
struct Editing {
    text: String,
    /// The text the cell had before the edit, so that a no-op edit can be skipped.
    original: String,
}

/// A message shown in the status bar.
#[derive(Clone, Debug, PartialEq)]
enum Notice {
    Info(String),
    Problem(String),
}

/// The whole application state.
pub struct Lattice {
    sheet: Sheet,
    selection: Selection,
    editing: Option<Editing>,
    scroll: Vector,
    viewport: Size,
    drag: Option<Drag>,
    modifiers: Modifiers,
    last_click: Option<(Instant, CellRef)>,
    notice: Option<Notice>,
    path: PathBuf,
}

impl Default for Lattice {
    fn default() -> Self {
        Lattice::new()
    }
}

impl Lattice {
    /// Open on a blank workbook: no example rows, nothing to delete first.
    ///
    /// A new spreadsheet should behave like a new spreadsheet — an empty grid and
    /// the cursor in A1 — so the first thing you do is type.
    pub fn new() -> Lattice {
        Lattice {
            sheet: Sheet::new(),
            selection: Selection::single(CellRef::new(0, 0)),
            editing: None,
            scroll: Vector::new(0.0, 0.0),
            // A reasonable guess until the first canvas event reports the real size.
            viewport: Size::new(1180.0, 620.0),
            drag: None,
            modifiers: Modifiers::default(),
            last_click: None,
            notice: None,
            path: PathBuf::from("lattice-sheet.json"),
        }
    }

    /// Alias for [`Lattice::new`], which is already empty.
    pub fn empty() -> Lattice {
        Lattice::new()
    }

    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    pub fn selection(&self) -> Selection {
        self.selection
    }

    pub fn scroll(&self) -> Vector {
        self.scroll
    }

    pub fn viewport(&self) -> Size {
        self.viewport
    }

    pub fn set_viewport(&mut self, size: Size) {
        self.viewport = size;
    }

    fn metrics(&self) -> Metrics {
        Metrics::new(self.scroll, self.viewport)
    }

    // --- editing surface -------------------------------------------------

    /// The text the formula bar shows for a cell: the formula source if it has one,
    /// otherwise the literal as it was typed.
    pub fn input_text(&self, cell: CellRef) -> String {
        match self.sheet.cell(cell).map(|stored| &stored.input) {
            Some(Input::Formula(formula)) => formula.source.clone(),
            Some(Input::Literal(value)) => value.as_text(),
            None => String::new(),
        }
    }

    /// What the formula bar should display right now.
    pub fn formula_text(&self) -> String {
        match &self.editing {
            Some(editing) => editing.text.clone(),
            None => self.input_text(self.selection.active),
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    // --- interaction -----------------------------------------------------

    /// Begin editing the active cell, seeded with `seed` when the user started by
    /// typing a character.
    fn begin_edit(&mut self, seed: Option<String>) -> Task<Message> {
        let cell = self.selection.active;
        let current = self.input_text(cell);
        let text = match seed {
            // Typing over a cell replaces its contents, as in every spreadsheet.
            Some(seed) => seed,
            None => current.clone(),
        };
        self.editing = Some(Editing { text, original: current });
        self.focus_editor(cell)
    }

    /// Focus the in-cell editor when the cell is on screen, otherwise the bar.
    fn focus_editor(&self, cell: CellRef) -> Task<Message> {
        let rect = self.metrics().cell_rect(cell);
        let visible = rect.x + rect.width >= HEADER_WIDTH
            && rect.y + rect.height >= HEADER_HEIGHT
            && rect.x <= self.viewport.width
            && rect.y <= self.viewport.height;
        let id = if visible { CELL_EDITOR } else { FORMULA_BAR };
        operate(focusable::focus(iced::widget::Id::new(id)))
    }

    fn unfocus() -> Task<Message> {
        operate(focusable::unfocus())
    }

    /// Accept the in-progress edit and optionally step to the next row.
    fn commit_edit(&mut self, advance: bool) -> Task<Message> {
        let Some(editing) = self.editing.take() else {
            return Task::none();
        };
        let cell = self.selection.active;
        if editing.text != editing.original {
            let report = self.sheet.set_input(cell, &editing.text);
            self.describe_recalc(cell, &report);
        }
        let task = if advance { self.move_selection(1, 0, false, false) } else { Task::none() };
        Task::batch([Self::unfocus(), task])
    }

    fn cancel_edit(&mut self) -> Task<Message> {
        self.editing = None;
        Self::unfocus()
    }

    /// Move the active cell, extending the selection when `extend` is set.
    fn move_selection(
        &mut self,
        row_delta: i32,
        col_delta: i32,
        extend: bool,
        jump: bool,
    ) -> Task<Message> {
        let active = self.selection.active;
        let target = if jump {
            self.jump_target(row_delta, col_delta)
        } else {
            CellRef::new(
                (active.row as i64 + row_delta as i64).clamp(0, MAX_ROWS as i64 - 1) as u32,
                (active.col as i64 + col_delta as i64).clamp(0, MAX_COLS as i64 - 1) as u32,
            )
        };
        if extend {
            self.selection.active = target;
        } else {
            self.selection = Selection::single(target);
        }
        let metrics = self.metrics();
        self.scroll = metrics.scroll_to_show(target, self.scroll);
        Task::none()
    }

    /// The target of a Ctrl+arrow jump: the edge of the used range, or of the sheet.
    fn jump_target(&self, row_delta: i32, col_delta: i32) -> CellRef {
        let active = self.selection.active;
        let used = self.sheet.used_bounds().unwrap_or_else(|| Bounds::single(active));
        CellRef::new(
            if row_delta > 0 { used.max_row.max(active.row) } else if row_delta < 0 { used.min_row.min(active.row) } else { active.row },
            if col_delta > 0 { used.max_col.max(active.col) } else if col_delta < 0 { used.min_col.min(active.col) } else { active.col },
        )
    }

    /// Select a whole row or column by clicking its gutter.
    fn select_column(&mut self, col: u32) {
        self.selection = Selection { anchor: CellRef::new(0, col), active: CellRef::new(MAX_ROWS - 1, col) };
    }

    fn select_row(&mut self, row: u32) {
        self.selection = Selection { anchor: CellRef::new(row, 0), active: CellRef::new(row, MAX_COLS - 1) };
    }

    fn pointer_pressed(&mut self, position: Point, viewport: Size) {
        self.viewport = viewport;
        let metrics = self.metrics();

        // The fill handle takes priority over the cell underneath it.
        if metrics.hits_fill_handle(self.selection.bounds(), position) {
            self.drag = Some(Drag::Filling(self.selection.bounds()));
            return;
        }

        // The gutters select whole rows and columns.
        if position.y < HEADER_HEIGHT && position.x >= HEADER_WIDTH {
            let x = position.x - HEADER_WIDTH + self.scroll.x;
            let col = (x / CELL_WIDTH).floor();
            if col >= 0.0 && col < MAX_COLS as f32 {
                self.select_column(col as u32);
            }
            return;
        }
        if position.x < HEADER_WIDTH && position.y >= HEADER_HEIGHT {
            let y = position.y - HEADER_HEIGHT + self.scroll.y;
            let row = (y / CELL_HEIGHT).floor();
            if row >= 0.0 && row < MAX_ROWS as f32 {
                self.select_row(row as u32);
            }
            return;
        }

        let Some(cell) = metrics.cell_at(position) else {
            return;
        };
        let now = Instant::now();
        let double_click = self
            .last_click
            .is_some_and(|(when, last)| last == cell && now.duration_since(when) <= DOUBLE_CLICK);
        self.last_click = Some((now, cell));

        if double_click {
            self.selection = Selection::single(cell);
            let _ = self.begin_edit(None);
            self.drag = None;
            return;
        }

        if self.modifiers.shift() {
            self.selection.active = cell;
        } else {
            self.selection = Selection::single(cell);
        }
        self.drag = Some(Drag::Selecting);
    }

    fn pointer_moved(&mut self, position: Point, viewport: Size) {
        self.viewport = viewport;
        let metrics = self.metrics();
        let Some(cell) = metrics.cell_at(position) else {
            return;
        };
        match self.drag {
            Some(Drag::Selecting) => {
                self.selection.active = cell;
            }
            Some(Drag::Filling(source)) => {
                // The preview is the source block grown to reach the cell under the
                // pointer, which is what a fill handle does.
                let dragged = padded_fill_target(source, Bounds::single(cell));
                self.drag = Some(Drag::Filling(dragged));
            }
            None => {}
        }
    }

    fn pointer_released(&mut self) {
        match self.drag.take() {
            Some(Drag::Filling(target)) => self.apply_fill(target),
            // Letting go of a range is a good moment to show what is in it.
            Some(Drag::Selecting) if !self.selection.is_single() => {
                self.notice = None;
                self.show_selection_summary();
            }
            Some(Drag::Selecting) => {}
            None => {}
        }
    }

    /// Apply a fill from the current selection into `target`.
    fn apply_fill(&mut self, target: Bounds) {
        let source = self.selection.bounds();
        if target == source {
            return;
        }
        let report = self.sheet.fill(source, target);
        let filled = target.len() - source.len().min(target.len());
        self.notice = Some(Notice::Info(format!(
            "filled {filled} cell{} (recalculated {} in {} level{})",
            if filled == 1 { "" } else { "s" },
            report.recalculated(),
            report.depth(),
            if report.depth() == 1 { "" } else { "s" },
        )));
        self.describe_cycles(&report.cycles);
        let metrics = self.metrics();
        self.scroll = metrics.scroll_to_show(last_cell(target), self.scroll);
    }

    fn show_selection_summary(&mut self) {
        let bounds = self.selection.bounds();
        if bounds.len() > SUMMARY_LIMIT {
            return;
        }
        let mut numbers = Vec::new();
        for cell in bounds.iter_cells() {
            if let Value::Number(n) = self.sheet.value(cell) {
                numbers.push(n);
            }
        }
        if numbers.len() >= 2 {
            let sum: f64 = numbers.iter().sum();
            let average = sum / numbers.len() as f64;
            self.notice = Some(Notice::Info(format!(
                "{} cells · {} numbers · sum {} · average {}",
                bounds.len(),
                numbers.len(),
                engine::format_number(sum),
                engine::format_number(average),
            )));
        }
    }

    /// Clear every cell that holds something inside the selection.
    fn clear_selection(&mut self) {
        let bounds = self.selection.bounds();
        let targets: Vec<CellRef> = self
            .sheet
            .iter_cells()
            .map(|(cell, _)| cell)
            .filter(|cell| bounds.contains(*cell))
            .collect();
        if targets.is_empty() {
            return;
        }
        for cell in targets {
            self.sheet.clear(cell);
        }
        self.notice = None;
    }

    fn describe_recalc(&mut self, cell: CellRef, report: &engine::RecalcReport) {
        self.describe_cycles(&report.cycles);
        if let Some(diagnostic) = self.sheet.formula_error(cell) {
            self.notice = Some(Notice::Problem(diagnostic.render(&self.input_text(cell))));
        }
    }

    fn describe_cycles(&mut self, cycles: &[CellRef]) {
        if cycles.is_empty() {
            return;
        }
        let names: Vec<String> = cycles.iter().take(4).map(|cell| cell.a1()).collect();
        let more = if cycles.len() > 4 { format!(" (+{})", cycles.len() - 4) } else { String::new() };
        self.notice = Some(Notice::Problem(format!(
            "circular reference: {}{more}",
            names.join(", ")
        )));
    }

    // --- files -----------------------------------------------------------

    fn save(&mut self) {
        match engine::io::save(&self.sheet, &self.path, "Sheet1") {
            Ok(()) => {
                self.notice = Some(Notice::Info(format!(
                    "saved {} cells to {}",
                    self.sheet.len(),
                    self.path.display()
                )));
            }
            Err(error) => self.notice = Some(Notice::Problem(format!("save failed: {error}"))),
        }
    }

    fn load(&mut self) {
        match engine::io::load(&self.path) {
            Ok(sheet) => {
                self.sheet = sheet;
                self.selection = Selection::single(CellRef::new(0, 0));
                self.editing = None;
                self.scroll = Vector::new(0.0, 0.0);
                self.notice =
                    Some(Notice::Info(format!("loaded {} cells from {}", self.sheet.len(), self.path.display())));
            }
            Err(error) => self.notice = Some(Notice::Problem(format!("open failed: {error}"))),
        }
    }

    /// Clear the workbook back to a single empty grid, leaving the file on disk
    /// untouched until the next explicit save.
    fn new_sheet(&mut self) {
        self.sheet = Sheet::new();
        self.selection = Selection::single(CellRef::new(0, 0));
        self.editing = None;
        self.scroll = Vector::new(0.0, 0.0);
        self.notice = Some(Notice::Info("new sheet".into()));
    }

    // --- messages --------------------------------------------------------

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Viewport(size) => {
                self.viewport = size;
                Task::none()
            }
            Message::Scrolled { delta, viewport } => {
                self.viewport = viewport;
                let metrics = self.metrics();
                self.scroll = metrics.clamp_scroll(self.scroll + delta);
                Task::none()
            }
            Message::PointerPressed { position, viewport } => {
                self.pointer_pressed(position, viewport);
                Task::none()
            }
            Message::PointerMoved { position, viewport } => {
                self.pointer_moved(position, viewport);
                Task::none()
            }
            Message::PointerReleased => {
                self.pointer_released();
                Task::none()
            }
            Message::Key { key, modifiers } => {
                self.modifiers = modifiers;
                self.key_pressed(key, modifiers)
            }
            Message::EditChanged(text) => {
                match &mut self.editing {
                    Some(editing) => editing.text = text,
                    // Typing in the formula bar while nothing is being edited
                    // starts an edit with what the bar now contains.
                    None => return self.begin_edit(Some(text)),
                }
                Task::none()
            }
            Message::EditSubmitted => self.commit_edit(true),
            Message::EditCancelled => self.cancel_edit(),
                    Message::Save => {
                self.save();
                Task::none()
            }
            Message::Load => {
                self.load();
                Task::none()
            }
            Message::NewSheet => {
                self.new_sheet();
                Task::none()
            }
        }
    }

    fn key_pressed(&mut self, key: Key, modifiers: Modifiers) -> Task<Message> {
        // File shortcuts work whether or not an edit is in progress.
        if modifiers.command() {
            if let Key::Character(character) = &key {
                match character.to_lowercase().as_str() {
                    "s" => {
                        self.save();
                        return Task::none();
                    }
                    "o" => {
                        self.load();
                        return Task::none();
                    }
                    _ => {}
                }
            }
        }

        if self.editing.is_some() {
            // With a text input focused, everything except Escape belongs to it.
            return if key == Key::Named(Named::Escape) {
                self.cancel_edit()
            } else {
                Task::none()
            };
        }

        let shift = modifiers.shift();
        match key {
            Key::Named(Named::ArrowUp) => self.move_selection(-1, 0, shift, modifiers.command()),
            Key::Named(Named::ArrowDown) => self.move_selection(1, 0, shift, modifiers.command()),
            Key::Named(Named::ArrowLeft) => self.move_selection(0, -1, shift, modifiers.command()),
            Key::Named(Named::ArrowRight) => self.move_selection(0, 1, shift, modifiers.command()),
            Key::Named(Named::Tab) => self.move_selection(0, if shift { -1 } else { 1 }, false, false),
            Key::Named(Named::Enter) => self.move_selection(if shift { -1 } else { 1 }, 0, false, false),
            Key::Named(Named::PageDown) => {
                let rows = self.visible_rows();
                self.move_selection(rows, 0, false, false)
            }
            Key::Named(Named::PageUp) => {
                let rows = self.visible_rows();
                self.move_selection(-rows, 0, false, false)
            }
            Key::Named(Named::Home) => {
                self.selection = Selection::single(CellRef::new(0, 0));
                self.scroll = Vector::new(0.0, 0.0);
                Task::none()
            }
            Key::Named(Named::End) => {
                let target = self.sheet.used_bounds().map_or_else(
                    || CellRef::new(0, 0),
                    |bounds| CellRef::new(bounds.max_row, bounds.max_col),
                );
                self.selection = Selection::single(target);
                let metrics = self.metrics();
                self.scroll = metrics.scroll_to_show(target, self.scroll);
                Task::none()
            }
            Key::Named(Named::Delete) | Key::Named(Named::Backspace) => {
                self.clear_selection();
                Task::none()
            }
            Key::Named(Named::Escape) => {
                self.selection = Selection::single(self.selection.active);
                Task::none()
            }
            Key::Named(Named::F2) => self.begin_edit(None),
            Key::Named(Named::Space) => self.begin_edit(Some(" ".to_string())),
            Key::Character(character) => {
                if modifiers.control() || modifiers.alt() {
                    Task::none()
                } else {
                    self.begin_edit(Some(character.to_string()))
                }
            }
            _ => Task::none(),
        }
    }

    fn visible_rows(&self) -> i32 {
        ((self.viewport.height - HEADER_HEIGHT) / CELL_HEIGHT).floor().max(1.0) as i32
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::event::listen_with(|event, status, _window| match event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if status == iced::event::Status::Captured {
                    // A focused text input owns the keystroke; only Escape is
                    // forwarded so that an edit can still be abandoned.
                    (key == Key::Named(Named::Escape)).then_some(Message::EditCancelled)
                } else {
                    Some(Message::Key { key, modifiers })
                }
            }
            iced::Event::Window(window::Event::Resized(size)) => Some(Message::Viewport(size)),
            _ => None,
        })
    }

    // --- view ------------------------------------------------------------

    pub fn view(&self) -> Element<'_, Message> {
        let metrics = self.metrics();
        let grid = canvas(GridProgram {
            sheet: &self.sheet,
            selection: self.selection.bounds(),
            active: self.selection.active,
            fill_preview: match self.drag {
                Some(Drag::Filling(target)) => Some(target),
                _ => None,
            },
            scroll: self.scroll,
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let body: Element<'_, Message> = match self.editor_overlay(&metrics) {
            Some(editor) => stack![grid, editor].into(),
            None => grid.into(),
        };

        column![
            self.top_bar(),
            self.formula_bar(),
            body,
            self.status_bar(),
        ]
        .into()
    }

    fn top_bar(&self) -> Element<'_, Message> {
        container(
            row![
                text("Lattice")
                    .size(15)
                    .font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT })
                    .color(theme::LEAF_DEEP),
                text("untitled spreadsheet").size(12).color(theme::INK_SOFT),
                Space::new().width(Length::Fill),
                button(text("New").size(12))
                    .padding([4, 10])
                    .style(theme::style::button_style)
                    .on_press(Message::NewSheet),
                button(text("Open").size(12))
                    .padding([4, 10])
                    .style(theme::style::button_style)
                    .on_press(Message::Load),
                button(text("Save").size(12))
                    .padding([4, 10])
                    .style(theme::style::button_style)
                    .on_press(Message::Save),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([6, 12])
        .style(theme::style::top_bar)
        .into()
    }

    fn formula_bar(&self) -> Element<'_, Message> {
        container(
            row![
                container(text(self.selection.active.a1()).size(12).color(theme::LEAF_DEEP))
                    .padding([3, 8])
                    .style(theme::style::reference_chip),
                text_input("value, or =formula", &self.formula_text())
                    .id(iced::widget::Id::new(FORMULA_BAR))
                    .size(13)
                    .padding(4)
                    .width(Length::Fill)
                    .style(theme::style::input_style)
                    .on_input(Message::EditChanged)
                    .on_submit(Message::EditSubmitted),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([6, 12])
        .style(theme::style::formula_bar)
        .into()
    }

    fn editor_overlay(&self, metrics: &Metrics) -> Option<Element<'_, Message>> {
        let editing = self.editing.as_ref()?;
        let rect = metrics.cell_rect(self.selection.active);
        let on_screen = rect.x + rect.width >= HEADER_WIDTH
            && rect.y + rect.height >= HEADER_HEIGHT
            && rect.x <= self.viewport.width
            && rect.y <= self.viewport.height;
        if !on_screen {
            return None;
        }

        // A text input placed exactly over the cell being edited, so the edit looks
        // like it is happening in the grid rather than in the formula bar.
        Some(
            container(
                text_input("", &editing.text)
                    .id(iced::widget::Id::new(CELL_EDITOR))
                    .size(13)
                    .padding(3)
                    .width(Length::Fixed(CELL_WIDTH + 1.0))
                    .style(theme::style::input_style)
                    .on_input(Message::EditChanged)
                    .on_submit(Message::EditSubmitted),
            )
            .padding(Padding { top: rect.y, left: rect.x, right: 0.0, bottom: 0.0 })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        )
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let (message, color) = match &self.notice {
            Some(Notice::Problem(problem)) => (problem.clone(), theme::CLAY),
            Some(Notice::Info(info)) => (info.clone(), theme::INK_SOFT),
            None => {
                let active = self.selection.active;
                match self.sheet.formula_error(active) {
                    Some(diagnostic) => (
                        diagnostic.render(&self.input_text(active)),
                        theme::CLAY,
                    ),
                    None if !self.selection.is_single() => {
                        (format!("{} selected", self.selection.bounds().len()), theme::INK_SOFT)
                    }
                    None => (
                        "type to edit · drag the corner to fill · Ctrl+S saves".to_string(),
                        theme::INK_SOFT,
                    ),
                }
            }
        };

        // A caret diagram lines the caret up under the offending character by
        // padding with spaces, which only holds in a monospace font.
        let font = if message.contains('\n') {
            Font::MONOSPACE
        } else {
            Font::DEFAULT
        };

        container(
            row![
                text(message).size(11).font(font).color(color),
                Space::new().width(Length::Fill),
                text(format!("{} cells · {}", self.sheet.len(), self.path.display()))
                    .size(11)
                    .color(theme::INK_SOFT),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([4, 12])
        .style(theme::style::status_bar)
        .into()
    }
}

/// Grow a fill target so that it always covers the source block.
fn padded_fill_target(source: Bounds, target: Bounds) -> Bounds {
    Bounds::new(
        CellRef::new(source.min_row.min(target.min_row), source.min_col.min(target.min_col)),
        CellRef::new(source.max_row.max(target.max_row), source.max_col.max(target.max_col)),
    )
}

/// A representative cell of a range, used for scroll-to-show.
fn last_cell(bounds: Bounds) -> CellRef {
    CellRef::new(bounds.max_row, bounds.max_col)
}

/// Messages the UI can produce.
#[derive(Debug, Clone)]
pub enum Message {
    /// The canvas was pressed; `viewport` is the canvas size at that moment.
    PointerPressed { position: Point, viewport: Size },
    PointerMoved { position: Point, viewport: Size },
    PointerReleased,
    Scrolled { delta: Vector, viewport: Size },
    /// The window (and therefore the canvas) changed size.
    Viewport(Size),
    Key { key: Key, modifiers: Modifiers },
    EditChanged(String),
    EditSubmitted,
    EditCancelled,
    Save,
    Load,
    NewSheet,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse_a1(a1).unwrap()
    }

    fn number(value: Value) -> f64 {
        match value {
            Value::Number(n) => n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    /// Floats are compared with a tolerance: `12 * 0.4` is `4.800000000000001` in
    /// IEEE-754 arithmetic, which is exactly the point of using `f64`.
    #[track_caller]
    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "expected {expected}, got {actual}");
    }

    /// A workbook with a few rows of data, for tests that need something to
    /// aggregate, delete, fill or navigate around.
    ///
    /// The application itself now opens blank, so any test that needs contents has
    /// to bring its own — this keeps that data out of the binary.
    fn populated() -> Lattice {
        let mut app = Lattice::new();
        let mut put = |a1: &str, text: &str| app.sheet.set_input(cell(a1), text);

        for (a1, text) in [
            ("A1", "Bed"),
            ("B1", "Crop"),
            ("C1", "Plants"),
            ("D1", "Yield"),
            ("E1", "Total"),
        ] {
            put(a1, text);
        }
        for (row, (bed, crop, plants, yield_)) in [
            ("1", "Sage", "12", "0.4"),
            ("2", "Mint", "20", "0.3"),
            ("3", "Thyme", "8", "0.6"),
        ]
        .into_iter()
        .enumerate()
        {
            let r = row + 2;
            put(&format!("A{r}"), bed);
            put(&format!("B{r}"), crop);
            put(&format!("C{r}"), plants);
            put(&format!("D{r}"), yield_);
            put(&format!("E{r}"), &format!("=C{r}*D{r}"));
        }
        put("A5", "Total");
        put("C5", "=SUM(C2:C4)");
        put("D5", "=AVERAGE(D2:D4)");
        put("E5", "=SUM(E2:E4)");
        put("A7", "Harvest");
        put("B7", "=IF(E5>10, \"plenty\", \"thin\")");
        put("A8", "Sage share");
        put("B8", "=CONCAT(\"Sage: \", C2/C5*100, \"%\")");
        put("A9", "Beds");
        put("B9", "=COUNT(C2:C4)");
        app
    }

    #[test]
    fn a_new_workbook_is_blank() {
        let app = Lattice::new();
        assert_eq!(app.sheet().len(), 0, "a new workbook should have no cells");
        assert_eq!(app.selection().active, CellRef::new(0, 0), "cursor starts in A1");
        assert_eq!(app.formula_text(), "", "the formula bar starts empty");
    }

    #[test]
    fn the_new_sheet_button_clears_the_grid() {
        let mut app = Lattice::new();
        let _ = app.update(Message::Key {
            key: Key::Character("5".into()),
            modifiers: Modifiers::default(),
        });
        let _ = app.update(Message::EditSubmitted);
        assert!(!app.sheet().is_empty(), "typing should create a cell");

        let _ = app.update(Message::NewSheet);
        assert_eq!(app.sheet().len(), 0, "New should clear the grid");
        assert_eq!(app.selection().active, CellRef::new(0, 0));
    }

    #[test]
    fn clicking_selects_and_dragging_extends() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);
        let metrics = Metrics::new(Vector::new(0.0, 0.0), viewport);

        let press = metrics.cell_rect(CellRef::new(2, 1));
        let _ = app.update(Message::PointerPressed {
            position: Point::new(press.x + 5.0, press.y + 5.0),
            viewport,
        });
        assert_eq!(app.selection(), Selection::single(CellRef::new(2, 1)));

        let drag_to = metrics.cell_rect(CellRef::new(5, 3));
        let _ = app.update(Message::PointerMoved {
            position: Point::new(drag_to.x + 5.0, drag_to.y + 5.0),
            viewport,
        });
        assert_eq!(app.selection().bounds(), Bounds::new(CellRef::new(2, 1), CellRef::new(5, 3)));
        let _ = app.update(Message::PointerReleased);
    }

    #[test]
    fn clicking_a_gutter_selects_the_whole_row_or_column() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);

        let _ = app.update(Message::PointerPressed { position: Point::new(10.0, HEADER_HEIGHT + 2.0), viewport });
        assert_eq!(app.selection().bounds(), Bounds::new(cell("A1"), CellRef::new(0, MAX_COLS - 1)));

        let _ = app.update(Message::PointerPressed { position: Point::new(HEADER_WIDTH + 2.0, 4.0), viewport });
        assert_eq!(app.selection().bounds(), Bounds::new(cell("A1"), CellRef::new(MAX_ROWS - 1, 0)));
    }

    #[test]
    fn typing_starts_an_edit_and_enter_commits_it() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::Key { key: Key::Character("5".into()), modifiers: Modifiers::default() });
        assert!(app.is_editing());
        assert_eq!(app.formula_text(), "5");

        let _ = app.update(Message::EditChanged("42".into()));
        let _ = app.update(Message::EditSubmitted);
        assert!(!app.is_editing());
        assert_eq!(app.sheet().value(cell("A1")), Value::Number(42.0));
        // Enter moves down, as in every spreadsheet.
        assert_eq!(app.selection().active, cell("A2"));
    }

    #[test]
    fn escape_abandons_an_edit_without_touching_the_sheet() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::Key { key: Key::Character("9".into()), modifiers: Modifiers::default() });
        let _ = app.update(Message::EditCancelled);
        assert!(!app.is_editing());
        assert_eq!(app.sheet().value(cell("A1")), Value::Empty);
    }

    #[test]
    fn arrow_keys_move_the_active_cell_and_shift_extends() {
        let mut app = Lattice::empty();
        let down = || Message::Key { key: Key::Named(Named::ArrowDown), modifiers: Modifiers::default() };
        let _ = app.update(down());
        let _ = app.update(down());
        assert_eq!(app.selection().active, cell("A3"));

        let shift_down = Message::Key {
            key: Key::Named(Named::ArrowDown),
            modifiers: Modifiers::SHIFT,
        };
        let _ = app.update(shift_down);
        assert_eq!(app.selection().bounds(), Bounds::new(cell("A3"), cell("A4")));
        assert_eq!(app.selection().anchor, cell("A3"));
    }

    #[test]
    fn the_active_cell_cannot_leave_the_sheet() {
        let mut app = Lattice::empty();
        let up = Message::Key { key: Key::Named(Named::ArrowUp), modifiers: Modifiers::default() };
        let left = Message::Key { key: Key::Named(Named::ArrowLeft), modifiers: Modifiers::default() };
        for _ in 0..5 {
            let _ = app.update(up.clone());
            let _ = app.update(left.clone());
        }
        assert_eq!(app.selection().active, cell("A1"));
    }

    #[test]
    fn delete_clears_the_selection_in_one_go() {
        let mut app = populated();
        app.selection = Selection { anchor: cell("C2"), active: cell("C4") };
        app.clear_selection();
        assert_eq!(app.sheet().value(cell("C2")), Value::Empty);
        assert_eq!(app.sheet().value(cell("C3")), Value::Empty);
        // The aggregate that read them was recalculated.
        assert_close(number(app.sheet().value(cell("C5"))), 0.0);
        // And nothing outside the selection was touched.
        assert_eq!(app.sheet().value(cell("B2")), Value::Text("Sage".into()));
    }

    #[test]
    fn deleting_a_formula_reports_its_replacement_value() {
        let mut app = populated();
        app.selection = Selection::single(cell("E2"));
        app.clear_selection();
        assert_eq!(app.sheet().value(cell("E2")), Value::Empty);
        assert_close(number(app.sheet().value(cell("E5"))), 10.8);
    }

    #[test]
    fn the_formula_bar_shows_the_formula_not_the_value() {
        let app = populated();
        assert_eq!(app.input_text(cell("E2")), "=C2*D2");
        assert_eq!(app.input_text(cell("B2")), "Sage");
        assert_eq!(app.input_text(cell("Z99")), "");
    }

    #[test]
    fn dropping_a_formula_into_an_empty_cell_reports_the_parse_error() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::Key { key: Key::Character("=".into()), modifiers: Modifiers::default() });
        let _ = app.update(Message::EditChanged("=1+*2".into()));
        let _ = app.update(Message::EditSubmitted);
        assert_eq!(app.sheet().value(cell("A1")), Value::Error(engine::ErrorKind::Parse));
        match &app.notice {
            Some(Notice::Problem(problem)) => assert!(problem.contains('^'), "{problem}"),
            other => panic!("expected a problem notice, got {other:?}"),
        }
    }

    #[test]
    fn a_circular_reference_is_reported_without_hanging() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::Key { key: Key::Character("=".into()), modifiers: Modifiers::default() });
        let _ = app.update(Message::EditChanged("=A1".into()));
        let _ = app.update(Message::EditSubmitted);
        assert_eq!(app.sheet().value(cell("A1")), Value::Error(engine::ErrorKind::Cycle));
        match &app.notice {
            Some(Notice::Problem(problem)) => assert!(problem.contains("circular")),
            other => panic!("expected a circular-reference notice, got {other:?}"),
        }
    }

    #[test]
    fn scrolling_is_clamped_and_uses_the_viewport_it_was_given() {
        let mut app = Lattice::empty();
        let viewport = Size::new(600.0, 400.0);
        let _ = app.update(Message::Scrolled { delta: Vector::new(0.0, -100.0), viewport });
        assert_eq!(app.scroll(), Vector::new(0.0, 0.0));
        let _ = app.update(Message::Scrolled { delta: Vector::new(0.0, 250.0), viewport });
        assert_eq!(app.scroll().y, 250.0);
    }

    #[test]
    fn a_fill_drag_copies_the_selection_with_relative_references() {
        let mut app = populated();
        // A fresh formula in the column beside the sample data, so the fill has
        // something to extend.
        app.sheet.set_input(cell("F2"), "=C2*10");
        app.selection = Selection::single(cell("F2"));
        let viewport = Size::new(900.0, 500.0);
        app.set_viewport(viewport);
        let metrics = Metrics::new(app.scroll(), viewport);

        // Grab the fill handle and drag it down to F4.
        let handle = metrics.fill_handle(app.selection().bounds());
        let _ = app.update(Message::PointerPressed {
            position: Point::new(handle.x + handle.width / 2.0, handle.y + handle.height / 2.0),
            viewport,
        });
        let target = metrics.cell_rect(cell("F4"));
        let _ = app.update(Message::PointerMoved {
            position: Point::new(target.x + 5.0, target.y + 5.0),
            viewport,
        });
        let _ = app.update(Message::PointerReleased);

        assert_eq!(app.sheet().formula_source(cell("F3")), Some("=C3*10"));
        assert_eq!(app.sheet().formula_source(cell("F4")), Some("=C4*10"));
        assert_close(number(app.sheet().value(cell("F2"))), 120.0);
        assert_close(number(app.sheet().value(cell("F3"))), 200.0);
        assert_close(number(app.sheet().value(cell("F4"))), 80.0);
    }

    #[test]
    fn a_fill_extends_across_columns_as_well_as_rows() {
        let mut app = Lattice::empty();
        app.sheet.set_input(cell("C5"), "=C2+C3");
        app.selection = Selection::single(cell("C5"));

        // Dragging the handle down and to the right fills a rectangle; every cell
        // is the source formula offset by its distance from the source.
        app.apply_fill(Bounds::new(cell("C5"), cell("D7")));

        assert_eq!(app.sheet().formula_source(cell("C5")), Some("=C2+C3"), "the source is untouched");
        assert_eq!(app.sheet().formula_source(cell("C6")), Some("=C3+C4"));
        assert_eq!(app.sheet().formula_source(cell("C7")), Some("=C4+C5"));
        assert_eq!(app.sheet().formula_source(cell("D6")), Some("=D3+D4"));
        assert_eq!(app.sheet().formula_source(cell("D7")), Some("=D4+D5"));
    }

    #[test]
    fn selecting_a_range_summarises_it() {
        let mut app = populated();
        app.selection = Selection { anchor: cell("C2"), active: cell("C4") };
        app.show_selection_summary();
        match &app.notice {
            Some(Notice::Info(info)) => {
                assert!(info.contains("sum 40"), "{info}");
                assert!(info.contains("average"), "{info}");
            }
            other => panic!("expected a summary, got {other:?}"),
        }
    }

    #[test]
    fn saving_and_loading_round_trips_through_the_file_system() {
        let mut app = Lattice::new();
        let dir = std::env::temp_dir().join(format!("lattice-app-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        app.path = dir.join("sheet.json");

        app.sheet.set_input(cell("A1"), "7");
        let _ = app.update(Message::Save);
        assert!(matches!(app.notice, Some(Notice::Info(_))));

        app.sheet.set_input(cell("A1"), "99");
        let _ = app.update(Message::Load);
        assert_eq!(app.sheet().value(cell("A1")), Value::Number(7.0), "load should restore the file");
        assert_eq!(app.selection(), Selection::single(cell("A1")));

        std::fs::remove_file(&app.path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn opening_a_missing_file_reports_a_problem_instead_of_panicking() {
        let mut app = Lattice::empty();
        app.path = PathBuf::from("/definitely/not/a/real/lattice-sheet.json");
        let _ = app.update(Message::Load);
        assert!(matches!(app.notice, Some(Notice::Problem(_))));
    }

    #[test]
    fn editing_an_off_screen_cell_still_focuses_something_usable() {
        let mut app = Lattice::empty();
        app.set_viewport(Size::new(600.0, 400.0));
        app.selection = Selection::single(CellRef::new(5000, 3));
        let _ = app.begin_edit(None);
        assert!(app.is_editing());
    }

    #[test]
    fn a_no_op_edit_does_not_write_to_the_sheet() {
        let mut app = populated();
        let before = number(app.sheet().value(cell("C5")));
        // `C5` is a formula cell, so an unmodified edit must be a no-op.
        app.selection = Selection::single(cell("C5"));
        let _ = app.begin_edit(None);
        assert_eq!(app.formula_text(), "=SUM(C2:C4)");
        let _ = app.update(Message::EditSubmitted);
        assert_close(number(app.sheet().value(cell("C5"))), before);
    }

    #[test]
    fn page_keys_scroll_by_a_screenful() {
        let mut app = Lattice::empty();
        app.set_viewport(Size::new(800.0, 400.0));
        let rows = app.visible_rows();
        assert!(rows > 5);
        let _ = app.update(Message::Key { key: Key::Named(Named::PageDown), modifiers: Modifiers::default() });
        assert_eq!(app.selection().active.row, rows as u32);
        assert!(app.scroll().y > 0.0, "the view should follow the active cell");
    }

    #[test]
    fn ctrl_arrow_jumps_to_the_edge_of_the_used_range() {
        let mut app = populated();
        app.selection = Selection::single(cell("A1"));
        let _ = app.update(Message::Key {
            key: Key::Named(Named::ArrowDown),
            modifiers: Modifiers::CTRL,
        });
        assert_eq!(app.selection().active, cell("A9"));
    }

    #[test]
    fn the_status_bar_reports_the_engine_error_for_the_active_cell() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::Key { key: Key::Character("=".into()), modifiers: Modifiers::default() });
        let _ = app.update(Message::EditChanged("=1+*2".into()));
        let _ = app.update(Message::EditSubmitted);
        app.notice = None;
        // Rendering is not available in tests, but the data path is.
        assert!(app.sheet().formula_error(cell("A1")).is_some());
        assert!(app.input_text(cell("A1")).starts_with('='));
    }
}
