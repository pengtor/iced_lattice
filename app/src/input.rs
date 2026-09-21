//! Keyboard and pointer handling: turning events into sheet operations.
//!
//! This is the Elm-style update loop and the small operations it drives — moving
//! the selection, editing a cell, filling a range, scrolling. It mutates
//! [`Lattice`] and asks the engine to do the actual work.

use std::time::{Duration, Instant};

use iced::advanced::widget::operation::focusable;
use iced::advanced::widget::operate;
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::window;
use iced::{Point, Size, Subscription, Task, Vector};

use engine::{Bounds, CellRef, Value, MAX_COLS, MAX_ROWS};

use crate::application::{CELL_EDITOR, FORMULA_BAR};
use crate::grid::{CELL_HEIGHT, CELL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH};
use crate::state::{Drag, Editing, Lattice, Message, Notice, Selection};
use crate::theme::ThemeMode;

/// Two presses on the same cell within this window start an edit.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);
/// A selection larger than this is not summarised in the status bar.
const SUMMARY_LIMIT: u64 = 50_000;

impl Lattice {
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
        self.notice = None;
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

    pub(crate) fn unfocus() -> Task<Message> {
        operate(focusable::unfocus())
    }

    /// Accept the in-progress edit and optionally step to the next row.
    pub(crate) fn commit_edit(&mut self, advance: bool) -> Task<Message> {
        let Some(editing) = self.editing.take() else {
            return Task::none();
        };
        let cell = self.selection.active;
        let report = (editing.text != editing.original)
            .then(|| self.sheet.set_input(cell, &editing.text));

        let task = if advance { self.move_selection(1, 0, false, false) } else { Task::none() };

        // Report the outcome *after* stepping: moving on clears the previous notice,
        // and a formula that failed to parse still deserves an explanation.
        if let Some(report) = report {
            self.describe_recalc(cell, &report);
        }
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
        // A message about the cell we just left would be misleading here.
        self.notice = None;
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

    /// Handle a press in the grid, returning the task needed to settle any edit it
    /// brings to an end.
    fn pointer_pressed(&mut self, position: Point, viewport: Size) -> Task<Message> {
        self.viewport = viewport;

        // Clicking the grid ends an edit in progress, committing it to the cell it
        // was started in, exactly as Excel does. This has to happen *before* the
        // selection moves, because `commit_edit` writes to the active cell — do it
        // afterwards and the typed text silently follows the click.
        let settled = self.commit_edit(false);

        let metrics = self.metrics();

        // The fill handle takes priority over the cell underneath it.
        if metrics.hits_fill_handle(self.selection.bounds(), position) {
            self.drag = Some(Drag::Filling(self.selection.bounds()));
            return settled;
        }

        // The gutters select whole rows and columns.
        if position.y < HEADER_HEIGHT && position.x >= HEADER_WIDTH {
            let x = position.x - HEADER_WIDTH + self.scroll.x;
            let col = (x / CELL_WIDTH).floor();
            if col >= 0.0 && col < MAX_COLS as f32 {
                self.notice = None;
                self.select_column(col as u32);
            }
            return settled;
        }
        if position.x < HEADER_WIDTH && position.y >= HEADER_HEIGHT {
            let y = position.y - HEADER_HEIGHT + self.scroll.y;
            let row = (y / CELL_HEIGHT).floor();
            if row >= 0.0 && row < MAX_ROWS as f32 {
                self.notice = None;
                self.select_row(row as u32);
            }
            return settled;
        }

        let Some(cell) = metrics.cell_at(position) else {
            return settled;
        };
        let now = Instant::now();
        let double_click = self
            .last_click
            .is_some_and(|(when, last)| last == cell && now.duration_since(when) <= DOUBLE_CLICK);
        self.last_click = Some((now, cell));

        // Whatever was reported about the previous cell does not describe this one.
        self.notice = None;

        if double_click {
            self.selection = Selection::single(cell);
            let edit = self.begin_edit(None);
            self.drag = None;
            return Task::batch([settled, edit]);
        }

        if self.modifiers.shift() {
            self.selection.active = cell;
        } else {
            self.selection = Selection::single(cell);
        }
        self.drag = Some(Drag::Selecting);
        settled
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
                self.pointer_pressed(position, viewport)
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
            Message::Save => self.save(),
            Message::SaveAs => self.save_as(),
            Message::Load => self.open(),
            Message::NewSheet => {
                self.new_sheet();
                Task::none()
            }
            Message::DialogChanged(text) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.text = text;
                    dialog.error = None;
                }
                Task::none()
            }
            Message::DialogSubmitted => self.submit_dialog(),
            Message::DialogCancelled => self.cancel_dialog(),
            Message::DialogPicked(name) => {
                if let Some(dialog) = &mut self.dialog {
                    dialog.text = name;
                    dialog.error = None;
                }
                self.submit_dialog()
            }
            Message::CycleTheme => {
                self.cycle_theme();
                Task::none()
            }
            Message::SystemTheme(mode) => {
                self.set_system_theme(mode);
                Task::none()
            }
        }
    }

    fn key_pressed(&mut self, key: Key, modifiers: Modifiers) -> Task<Message> {
        // The naming prompt is modal: until it is answered, the keyboard belongs to
        // it, so no stray arrow key can move a selection the user cannot see.
        if self.dialog.is_some() {
            return match key {
                Key::Named(Named::Escape) => self.cancel_dialog(),
                Key::Named(Named::Enter) => self.submit_dialog(),
                _ => Task::none(),
            };
        }

        // File shortcuts work whether or not an edit is in progress.
        if modifiers.command() {
            if let Key::Character(character) = &key {
                match character.to_lowercase().as_str() {
                    "s" => {
                        // Shift turns the ordinary save into a rename.
                        return if modifiers.shift() { self.save_as() } else { self.save() };
                    }
                    "o" => return self.open(),
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
                self.notice = None;
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
                self.notice = None;
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
        // Keyboard and window events,
        let input = iced::event::listen_with(|event, status, _window| match event {
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
        });

        // ...plus the desktop's own light/dark setting. iced broadcasts the current
        // mode when a window appears and again whenever the desktop changes it, so
        // this one subscription covers both startup and live changes. `boot` also
        // asks outright, in case the first broadcast lands before this subscription
        // exists; the two are idempotent, so overlapping costs nothing.
        //
        // On Linux this is the *only* working route, and it is not the obvious one.
        // winit reports a change of desktop theme only on macOS, Windows and the
        // web — its X11 backend answers `None` when asked directly, and Wayland
        // reflects what the application itself requested, not the desktop. So the
        // signal here comes from iced's own `linux-theme-detection` path, which
        // watches `org.freedesktop.appearance`/`color-scheme` over the
        // xdg-desktop-portal and streams changes back. That feature is part of
        // iced's `default` set but *not* of ours, because the app hand-picks its
        // features (`default-features = false`); with it switched off, `System`
        // silently resolves to light on Linux forever. Hence listing it explicitly
        // in the workspace `iced` dependency — see Cargo.toml, which also records
        // what it costs (it pulls in `zbus`).
        //
        // Deliberate limits, so nobody reads them as half-finished work: with no
        // portal answering, `mundy` gives up after 200ms and the answer is
        // `Mode::None`; and a desktop that genuinely expresses no preference also
        // reports `NoPreference`. Both land on light (see `ThemeMode::from_iced`),
        // which is the mode this app has always drawn — a defensible default, not
        // a detected one. The manual toggle covers every case either way.
        let system_theme = iced::system::theme_changes()
            .map(|mode| Message::SystemTheme(ThemeMode::from_iced(mode)));

        Subscription::batch([input, system_theme])
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Metrics;
    use crate::state::test_support::{assert_close, cell, number, populated};
    use crate::theme::ThemePreference;

    #[test]
    fn clicking_another_cell_commits_the_edit_rather_than_moving_it() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);
        let metrics = Metrics::new(Vector::new(0.0, 0.0), viewport);

        // Start typing in A1, then click C3 instead of pressing Enter.
        let _ = app.update(Message::Key {
            key: Key::Character("5".into()),
            modifiers: Modifiers::default(),
        });
        assert!(app.is_editing());

        let target = metrics.cell_rect(cell("C3"));
        let _ = app.update(Message::PointerPressed {
            position: Point::new(target.x + 5.0, target.y + 5.0),
            viewport,
        });

        assert!(!app.is_editing(), "clicking away ends the edit");
        assert_eq!(app.selection().active, cell("C3"), "and still moves the cursor");
        assert_eq!(app.sheet().value(cell("A1")), Value::Number(5.0), "the typing belongs to A1");
        assert_eq!(app.sheet().value(cell("C3")), Value::Empty, "not to the cell clicked");
    }

    #[test]
    fn a_diagnostic_does_not_follow_the_cursor() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);
        let metrics = Metrics::new(Vector::new(0.0, 0.0), viewport);

        // A formula that cannot parse: the cell keeps the error and the status bar
        // explains it with a caret diagram.
        let _ = app.update(Message::Key {
            key: Key::Character("=".into()),
            modifiers: Modifiers::default(),
        });
        let _ = app.update(Message::EditChanged("=SUM(".into()));
        let _ = app.update(Message::EditSubmitted);
        assert_eq!(app.sheet().value(cell("A1")), Value::Error(engine::ErrorKind::Parse));
        assert!(matches!(app.notice, Some(Notice::Problem(_))), "the bad formula explains itself");

        // Clicking a different cell must not carry that message along with it.
        let target = metrics.cell_rect(cell("B2"));
        let _ = app.update(Message::PointerPressed {
            position: Point::new(target.x + 5.0, target.y + 5.0),
            viewport,
        });
        assert_eq!(app.selection().active, cell("B2"));
        assert!(app.notice.is_none(), "the diagnostic must not follow the cursor");

        // The same goes for stepping away with the keyboard.
        let _ = app.update(Message::Key {
            key: Key::Character("=".into()),
            modifiers: Modifiers::default(),
        });
        let _ = app.update(Message::EditChanged("=1+".into()));
        let _ = app.update(Message::EditSubmitted);
        assert!(matches!(app.notice, Some(Notice::Problem(_))), "B2 is the bad one now");

        let _ = app.update(Message::Key {
            key: Key::Named(Named::ArrowRight),
            modifiers: Modifiers::default(),
        });
        assert_eq!(app.selection().active, cell("C3"), "Enter stepped to B3, the arrow to C3");
        assert!(app.notice.is_none(), "an arrow key should clear it too");
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

    /// One click is one step: System → Light → Dark → System, and no step is missed.
    ///
    /// Driven through `update` rather than by calling `cycle_theme`, because the
    /// cycle itself was never the suspect — a message delivered twice, or a read
    /// taken before the write earlier in the same step, would be invisible to a test
    /// that calls the cycle directly. This asserts what a user's three clicks
    /// actually produce.
    #[test]
    fn the_theme_toggle_advances_one_preference_per_click() {
        let dir = std::env::temp_dir().join(format!("lattice-toggle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut app = Lattice::empty();
        // Cycling writes the preference down, so keep it out of the working dir.
        app.folder = dir.clone();
        // The desktop this app runs against reports light, which is exactly the case
        // that used to look broken: `System` and `Light` draw the same window.
        app.set_system_theme(ThemeMode::Light);
        assert_eq!(app.theme_preference(), ThemePreference::System);

        let mut seen = Vec::new();
        for _ in 0..3 {
            let _ = app.update(Message::CycleTheme);
            seen.push((app.theme_preference(), app.theme_label()));
        }

        assert_eq!(
            seen,
            vec![
                (ThemePreference::Light, "Light".to_string()),
                (ThemePreference::Dark, "Dark".to_string()),
                (ThemePreference::System, "System · light".to_string()),
            ],
            "a click must land on the next preference, once per click"
        );

        // And the step that cannot repaint the window still says what it did: the
        // label is on screen even when the palette has nothing to change.
        assert_eq!(app.theme_mode(), ThemeMode::Light, "system is following the desktop");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The one click that cannot repaint the window, spelled out on its own.
    ///
    /// On a desktop that reports light, `System` and `Light` resolve to the same mode,
    /// so the first click out of System takes a step and leaves every colour exactly
    /// where it was. That is the intended semantics rather than a dropped click: the two
    /// preferences are genuinely different, and genuinely look alike on that desktop.
    /// What carries the click is the label — and, because the label alone proved too
    /// quiet to notice, the toggle's own styling; see
    /// `theme::tests::the_toggle_only_looks_committed_once_a_mode_has_been_pinned`.
    ///
    /// What must *not* happen is the cycle skipping over Light to Dark to manufacture a
    /// repaint. That is asserted here, so the cheap "fix" cannot come back.
    #[test]
    fn one_click_from_system_on_a_light_desktop_pins_light() {
        let dir = std::env::temp_dir().join(format!("lattice-toggle-light-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut app = Lattice::empty();
        app.folder = dir.clone();
        // The desktop is light, which is the case the bug report describes.
        app.set_system_theme(ThemeMode::Light);

        // Before: following a light desktop. The label names the preference *and* what it
        // resolved to, so `System` is never mistaken for a mode of its own.
        assert_eq!(app.theme_label(), "System · light");
        assert_eq!(app.theme_mode(), ThemeMode::Light);

        let _ = app.update(Message::CycleTheme);

        // The step was taken...
        assert_eq!(app.theme_preference(), ThemePreference::Light);
        // ...without skipping to Dark in search of a visible change...
        assert_ne!(app.theme_preference(), ThemePreference::Dark, "no step may be skipped");
        // ...and the window is unchanged, because those two preferences paint alike here.
        assert_eq!(app.theme_mode(), ThemeMode::Light, "system and pinned-light agree");
        // Which leaves the button as the thing that has to say the click landed.
        assert_eq!(app.theme_label(), "Light", "the click still has to say something");

        // And the cycle carries on one step per click from there.
        let _ = app.update(Message::CycleTheme);
        assert_eq!(app.theme_preference(), ThemePreference::Dark);

        std::fs::remove_dir_all(&dir).ok();
    }
}
