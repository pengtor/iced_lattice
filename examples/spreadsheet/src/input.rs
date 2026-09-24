use std::time::{Duration, Instant};

use iced::advanced::widget::operation::focusable;
use iced::advanced::widget::operation::text_input as text_ops;
use iced::advanced::widget::operate;
use iced::keyboard::key::Named;
use iced::keyboard::{Key, Modifiers};
use iced::window;
use iced::{Subscription, Task, Vector};

use engine::{CellRef, Value};

use crate::application::{CELL_EDITOR, FORMULA_BAR, NAME_BOX};
use crate::grid::{
    Bounds, CellRef as GridCell, GridEvent, Hooks, Selection, CELL_HEIGHT, HEADER_HEIGHT,
    HEADER_WIDTH,
};
use crate::model::{sheet_bounds, sheet_cell, SheetView};
use crate::state::{Editing, Lattice, Message, NameBox, Notice};
use crate::theme::ThemeMode;

const SUMMARY_LIMIT: u64 = 50_000;
// How long a refused name stays red
const NAME_FLASH: Duration = Duration::from_millis(700);

// The grid's hooks. Each one is answered with a Message rather than acted on
// here, so the decision lands back in `update` where the sheet is reachable.

fn edit_requested(cell: GridCell) -> Message {
    Message::EditRequested(cell)
}

fn fill_committed(source: Bounds, target: Bounds) -> Message {
    Message::FillCommitted { source, target }
}

fn selection_settled(bounds: Bounds) -> Message {
    Message::SelectionSettled(bounds)
}

fn clear_requested(bounds: Bounds) -> Message {
    Message::ClearRequested(bounds)
}

const HOOKS: Hooks<Message> = Hooks {
    on_edit_requested: edit_requested,
    on_fill_committed: fill_committed,
    on_selection_settled: selection_settled,
    on_clear_requested: clear_requested,
};

impl Lattice {

    fn begin_edit(&mut self, seed: Option<String>) -> Task<Message> {
        let cell = sheet_cell(self.grid.selection.active);
        let current = self.input_text(cell);
        let text = match seed {
            Some(seed) => seed,
            None => current.clone(),
        };
        self.editing = Some(Editing { text, original: current });
        self.notice = None;
        self.focus_editor(self.grid.selection.active)
    }

    fn focus_editor(&self, cell: GridCell) -> Task<Message> {
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

    pub(crate) fn commit_edit(&mut self, advance: bool) -> Task<Message> {
        let Some(editing) = self.editing.take() else {
            return Task::none();
        };
        let cell = sheet_cell(self.grid.selection.active);
        let report = (editing.text != editing.original)
            .then(|| self.sheet.set_input(cell, &editing.text));

        // Report after stepping, since the move clears any previous notice
        if advance {
            let _ = self.step(1, 0);
        }

        if let Some(report) = report {
            self.describe_recalc(cell, &report);
        }
        Task::batch([Self::unfocus(), Task::none()])
    }

    fn cancel_edit(&mut self) -> Task<Message> {
        self.editing = None;
        Self::unfocus()
    }

    // Opening the box commits a pending edit instead
    fn open_name_box(&mut self) -> Task<Message> {
        // The editor's unfocus is moot: it is leaving the tree
        let _ = self.commit_edit(false);
        self.name_box = Some(NameBox { text: self.grid.selection.active.a1(), rejected: false });
        Self::focus_name_box()
    }

    fn focus_name_box() -> Task<Message> {
        let id = iced::widget::Id::new(NAME_BOX);
        Task::batch([
            operate(focusable::focus(id.clone())),
            operate(text_ops::select_all(id)),
        ])
    }

    fn cancel_name_box(&mut self) -> Task<Message> {
        self.name_box = None;
        Self::unfocus()
    }

    fn submit_name_box(&mut self) -> Task<Message> {
        let Some(text) = self.name_box.as_ref().map(|name_box| name_box.text.clone()) else {
            return Task::none();
        };
        let Some(target) = parse_target(&text) else {
            if let Some(name_box) = self.name_box.as_mut() {
                name_box.rejected = true;
            }
            return Task::none();
        };
        self.grid.selection = target;
        self.notice = None;
        let metrics = self.metrics();
        self.grid.scroll = metrics.scroll_to_show(target.active, self.grid.scroll);
        self.name_box = None;
        Self::unfocus()
    }

    /// Hand a grid event to the controller and answer its hooks. Hit-testing,
    /// drag state and double-click timing all live in the controller.
    fn route(&mut self, event: GridEvent) -> Task<Message> {
        let extend = self.modifiers.shift();
        let messages =
            self.grid.handle(&SheetView(&self.sheet), &event, extend, Instant::now(), &HOOKS);
        self.answer(messages)
    }

    /// Run the hooks' messages back through `update`, so the grid decides
    /// *that* something happened and the app decides what it means in exactly
    /// one place.
    fn answer(&mut self, messages: Vec<Message>) -> Task<Message> {
        let mut tasks = Vec::with_capacity(messages.len());
        for message in messages {
            tasks.push(self.update(message));
        }
        Task::batch(tasks)
    }

    fn step(&mut self, row_delta: i32, col_delta: i32) -> Task<Message> {
        self.move_active(row_delta, col_delta, false, false)
    }

    /// Keyboard movement, which no [`GridEvent`] carries.
    ///
    /// [`GridEvent`]: crate::grid::GridEvent
    fn move_active(
        &mut self,
        row_delta: i32,
        col_delta: i32,
        extend: bool,
        jump: bool,
    ) -> Task<Message> {
        self.notice = None;
        self.grid.move_selection(
            &SheetView(&self.sheet),
            self.viewport,
            row_delta,
            col_delta,
            extend,
            jump,
        );
        Task::none()
    }

    /// Ask the controller what "clear" means and let the hook come back here.
    fn clear_selection(&mut self) -> Task<Message> {
        self.answer(self.grid.request_clear(&HOOKS))
    }

    /// The clear hook: the engine knows which of its cells are populated, so
    /// walking them is the host's half of the job.
    fn clear_range(&mut self, bounds: Bounds) {
        let bounds = sheet_bounds(bounds);
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

    /// The fill hook. The grid works out the two blocks; shifting relative
    /// references and recalculating is the spreadsheet half of "fill".
    fn apply_fill(&mut self, source: Bounds, target: Bounds) {
        let source = sheet_bounds(source);
        let target = sheet_bounds(target);
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
        let last = GridCell::new(target.max_row, target.max_col);
        self.grid.scroll = metrics.scroll_to_show(last, self.grid.scroll);
    }

    fn show_selection_summary(&mut self, bounds: Bounds) {
        let bounds = sheet_bounds(bounds);
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

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Viewport(size) => {
                self.viewport = size;
                Task::none()
            }
            Message::Scrolled { delta, viewport } => {
                self.viewport = viewport;
                self.route(GridEvent::Scrolled { delta, viewport })
            }
            Message::PointerPressed { position, viewport } => {
                self.viewport = viewport;
                // Commit before moving, or the edit text follows the click
                let settled = self.commit_edit(false);
                self.notice = None;
                let routed = self.route(GridEvent::PointerPressed { position, viewport });
                Task::batch([settled, routed])
            }
            Message::PointerMoved { position, viewport } => {
                self.viewport = viewport;
                self.route(GridEvent::PointerMoved { position, viewport })
            }
            Message::PointerReleased => self.route(GridEvent::PointerReleased),
            Message::Key { key, modifiers } => {
                self.modifiers = modifiers;
                self.key_pressed(key, modifiers)
            }
            // The grid asked for the editor. It has already put the cursor on
            // the cell it named, so opening the editor is all that is left.
            Message::EditRequested(_) => self.begin_edit(None),
            Message::FillCommitted { source, target } => {
                self.apply_fill(source, target);
                Task::none()
            }
            Message::SelectionSettled(bounds) => {
                self.notice = None;
                self.show_selection_summary(bounds);
                Task::none()
            }
            Message::ClearRequested(bounds) => {
                self.clear_range(bounds);
                Task::none()
            }
            Message::EditChanged(text) => {
                match &mut self.editing {
                    Some(editing) => editing.text = text,
                    None => return self.begin_edit(Some(text)),
                }
                Task::none()
            }
            Message::EditSubmitted => self.commit_edit(true),
            Message::EditCancelled => {
                // Focused input forwards only Escape, maybe the name box
                if self.name_box.is_some() {
                    self.cancel_name_box()
                } else {
                    self.cancel_edit()
                }
            }
            Message::NameBoxActivated => self.open_name_box(),
            Message::NameBoxChanged(text) => {
                if let Some(name_box) = self.name_box.as_mut() {
                    name_box.text = text;
                    name_box.rejected = false;
                }
                Task::none()
            }
            Message::NameBoxSubmitted => self.submit_name_box(),
            Message::NameBoxCancelled => self.cancel_name_box(),
            Message::NameBoxFlashEnded => {
                if let Some(name_box) = self.name_box.as_mut() {
                    name_box.rejected = false;
                }
                Task::none()
            }
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
        if self.dialog.is_some() {
            return match key {
                Key::Named(Named::Escape) => self.cancel_dialog(),
                Key::Named(Named::Enter) => self.submit_dialog(),
                _ => Task::none(),
            };
        }

        // Shortcuts work while editing, so checked before the editor
        if modifiers.command() {
            if let Key::Character(character) = &key {
                match character.to_lowercase().as_str() {
                    "s" => {
                        return if modifiers.shift() { self.save_as() } else { self.save() };
                    }
                    "o" => return self.open(),
                    "g" => return self.open_name_box(),
                    _ => {}
                }
            }
        }

        // Typing goes to the box itself; only Escape lands here
        if self.name_box.is_some() {
            return if key == Key::Named(Named::Escape) {
                self.cancel_name_box()
            } else {
                Task::none()
            };
        }

        if self.editing.is_some() {
            return if key == Key::Named(Named::Escape) {
                self.cancel_edit()
            } else {
                Task::none()
            };
        }

        let shift = modifiers.shift();
        match key {
            Key::Named(Named::ArrowUp) => self.move_active(-1, 0, shift, modifiers.command()),
            Key::Named(Named::ArrowDown) => self.move_active(1, 0, shift, modifiers.command()),
            Key::Named(Named::ArrowLeft) => self.move_active(0, -1, shift, modifiers.command()),
            Key::Named(Named::ArrowRight) => self.move_active(0, 1, shift, modifiers.command()),
            Key::Named(Named::Tab) => self.move_active(0, if shift { -1 } else { 1 }, false, false),
            Key::Named(Named::Enter) => self.move_active(if shift { -1 } else { 1 }, 0, false, false),
            Key::Named(Named::PageDown) => {
                let rows = self.visible_rows();
                self.move_active(rows, 0, false, false)
            }
            Key::Named(Named::PageUp) => {
                let rows = self.visible_rows();
                self.move_active(-rows, 0, false, false)
            }
            Key::Named(Named::Home) => {
                self.grid.selection = Selection::single(GridCell::new(0, 0));
                self.grid.scroll = Vector::new(0.0, 0.0);
                self.notice = None;
                Task::none()
            }
            Key::Named(Named::End) => {
                let target = self.sheet.used_bounds().map_or_else(
                    || GridCell::new(0, 0),
                    |bounds| GridCell::new(bounds.max_row, bounds.max_col),
                );
                self.grid.selection = Selection::single(target);
                let metrics = self.metrics();
                self.grid.scroll = metrics.scroll_to_show(target, self.grid.scroll);
                self.notice = None;
                Task::none()
            }
            Key::Named(Named::Delete) | Key::Named(Named::Backspace) => self.clear_selection(),
            Key::Named(Named::Escape) => {
                self.grid.selection = Selection::single(self.grid.selection.active);
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
        let input = iced::event::listen_with(|event, status, _window| match event {
            iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                if status == iced::event::Status::Captured {
                    // A focused text input owns keys; only Escape is forwarded
                    (key == Key::Named(Named::Escape)).then_some(Message::EditCancelled)
                } else {
                    Some(Message::Key { key, modifiers })
                }
            }
            iced::Event::Window(window::Event::Resized(size)) => Some(Message::Viewport(size)),
            _ => None,
        });

        // Linux theme needs iced's linux-theme-detection feature (see Cargo.toml)
        let system_theme = iced::system::theme_changes()
            .map(|mode| Message::SystemTheme(ThemeMode::from_iced(mode)));

        // Ticks only while a refusal shows, then clears itself
        let flash = self
            .name_box
            .as_ref()
            .is_some_and(|name_box| name_box.rejected)
            .then(|| iced::time::every(NAME_FLASH).map(|_| Message::NameBoxFlashEnded));

        Subscription::batch([input, system_theme].into_iter().chain(flash))
    }
}

// "B12" or "B2:D10". A1 notation itself lives in the widget; which corner of a
// range ends up active is the app's policy, so that decision stays here.
fn parse_target(text: &str) -> Option<Selection> {
    let bounds = Bounds::parse_a1(text)?;
    Some(Selection {
        anchor: GridCell::new(bounds.max_row, bounds.max_col),
        active: GridCell::new(bounds.min_row, bounds.min_col),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Metrics;
    use crate::state::test_support::{assert_close, cell, gbounds, gcell, number, populated};
    use crate::theme::ThemePreference;
    use iced::{Point, Size};

    fn viewport() -> Size {
        Size::new(900.0, 500.0)
    }

    #[test]
    fn clicking_another_cell_commits_the_edit_rather_than_moving_it() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);
        let metrics = Metrics::new(Vector::new(0.0, 0.0), viewport, crate::model::dims());

        let _ = app.update(Message::Key {
            key: Key::Character("5".into()),
            modifiers: Modifiers::default(),
        });
        assert!(app.is_editing());

        let target = metrics.cell_rect(gcell("C3"));
        let _ = app.update(Message::PointerPressed {
            position: Point::new(target.x + 5.0, target.y + 5.0),
            viewport,
        });

        assert!(!app.is_editing(), "clicking away ends the edit");
        assert_eq!(app.selection().active, gcell("C3"), "and still moves the cursor");
        assert_eq!(app.sheet().value(cell("A1")), Value::Number(5.0), "the typing belongs to A1");
        assert_eq!(app.sheet().value(cell("C3")), Value::Empty, "not to the cell clicked");
    }

    #[test]
    fn double_clicking_a_cell_opens_the_editor_through_the_hook() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);
        let metrics = Metrics::new(Vector::new(0.0, 0.0), viewport, crate::model::dims());
        let target = metrics.cell_rect(gcell("C3"));
        let position = Point::new(target.x + 5.0, target.y + 5.0);

        // The first press only moves the cursor
        let _ = app.update(Message::PointerPressed { position, viewport });
        assert!(!app.is_editing(), "one click selects, it does not edit");
        assert_eq!(app.selection().active, gcell("C3"));

        // The second, inside the double-click window, asks for the editor
        let _ = app.update(Message::PointerPressed { position, viewport });
        assert!(app.is_editing(), "the controller's edit-requested hook opened the editor");
        assert_eq!(app.selection().active, gcell("C3"), "on the cell that was clicked");
    }

    #[test]
    fn a_diagnostic_does_not_follow_the_cursor() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);
        let metrics = Metrics::new(Vector::new(0.0, 0.0), viewport, crate::model::dims());

        let _ = app.update(Message::Key {
            key: Key::Character("=".into()),
            modifiers: Modifiers::default(),
        });
        let _ = app.update(Message::EditChanged("=SUM(".into()));
        let _ = app.update(Message::EditSubmitted);
        assert_eq!(app.sheet().value(cell("A1")), Value::Error(engine::ErrorKind::Parse));
        assert!(matches!(app.notice, Some(Notice::Problem(_))), "the bad formula explains itself");

        let target = metrics.cell_rect(gcell("B2"));
        let _ = app.update(Message::PointerPressed {
            position: Point::new(target.x + 5.0, target.y + 5.0),
            viewport,
        });
        assert_eq!(app.selection().active, gcell("B2"));
        assert!(app.notice.is_none(), "the diagnostic must not follow the cursor");

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
        assert_eq!(app.selection().active, gcell("C3"), "Enter stepped to B3, the arrow to C3");
        assert!(app.notice.is_none(), "an arrow key should clear it too");
    }

    #[test]
    fn a_shift_click_extends_from_the_anchor_through_the_app() {
        let mut app = Lattice::empty();
        let viewport = Size::new(1000.0, 600.0);
        let metrics = Metrics::new(Vector::new(0.0, 0.0), viewport, crate::model::dims());

        // The app learns the shift state off the last key event, as before
        let _ = app.update(Message::Key {
            key: Key::Named(Named::Shift),
            modifiers: Modifiers::SHIFT,
        });

        let target = metrics.cell_rect(gcell("C3"));
        let _ = app.update(Message::PointerPressed {
            position: Point::new(target.x + 5.0, target.y + 5.0),
            viewport,
        });

        assert_eq!(app.selection().anchor, gcell("A1"), "the anchor did not move");
        assert_eq!(app.selection().bounds(), gbounds("A1", "C3"), "but the click extended");
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
        assert_eq!(app.selection().active, gcell("A2"));
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
    fn delete_clears_the_selection_in_one_go() {
        let mut app = populated();
        app.grid.selection = Selection { anchor: gcell("C2"), active: gcell("C4") };
        let _ = app.clear_selection();
        assert_eq!(app.sheet().value(cell("C2")), Value::Empty);
        assert_eq!(app.sheet().value(cell("C3")), Value::Empty);
        assert_close(number(app.sheet().value(cell("C5"))), 0.0);
        assert_eq!(app.sheet().value(cell("B2")), Value::Text("Sage".into()));
    }

    #[test]
    fn deleting_a_formula_reports_its_replacement_value() {
        let mut app = populated();
        app.grid.selection = Selection::single(gcell("E2"));
        let _ = app.clear_selection();
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
    fn a_fill_drag_copies_the_selection_with_relative_references() {
        let mut app = populated();
        app.sheet.set_input(cell("F2"), "=C2*10");
        app.grid.selection = Selection::single(gcell("F2"));
        let viewport = Size::new(900.0, 500.0);
        app.set_viewport(viewport);
        let metrics = Metrics::new(app.scroll(), viewport, crate::model::dims());

        let handle = metrics.fill_handle(app.selection().bounds());
        let _ = app.update(Message::PointerPressed {
            position: Point::new(handle.x + handle.width / 2.0, handle.y + handle.height / 2.0),
            viewport,
        });
        let target = metrics.cell_rect(gcell("F4"));
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
        app.grid.selection = Selection::single(gcell("C5"));

        app.apply_fill(gbounds("C5", "C5"), gbounds("C5", "D7"));

        assert_eq!(app.sheet().formula_source(cell("C5")), Some("=C2+C3"), "the source is untouched");
        assert_eq!(app.sheet().formula_source(cell("C6")), Some("=C3+C4"));
        assert_eq!(app.sheet().formula_source(cell("C7")), Some("=C4+C5"));
        assert_eq!(app.sheet().formula_source(cell("D6")), Some("=D3+D4"));
        assert_eq!(app.sheet().formula_source(cell("D7")), Some("=D4+D5"));
    }

    #[test]
    fn dragging_a_range_summarises_it_through_the_hook() {
        let mut app = populated();
        let viewport = Size::new(900.0, 500.0);
        app.set_viewport(viewport);
        let metrics = Metrics::new(app.scroll(), viewport, crate::model::dims());

        let from = metrics.cell_rect(gcell("C2"));
        let to = metrics.cell_rect(gcell("C4"));
        let _ = app.update(Message::PointerPressed {
            position: Point::new(from.x + 5.0, from.y + 5.0),
            viewport,
        });
        let _ = app.update(Message::PointerMoved {
            position: Point::new(to.x + 5.0, to.y + 5.0),
            viewport,
        });
        let _ = app.update(Message::PointerReleased);

        assert_eq!(app.selection().bounds(), gbounds("C2", "C4"));
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
        app.grid.selection = Selection::single(GridCell::new(5000, 3));
        let _ = app.begin_edit(None);
        assert!(app.is_editing());
    }

    #[test]
    fn a_no_op_edit_does_not_write_to_the_sheet() {
        let mut app = populated();
        let before = number(app.sheet().value(cell("C5")));
        app.grid.selection = Selection::single(gcell("C5"));
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
    fn the_status_bar_reports_the_engine_error_for_the_active_cell() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::Key { key: Key::Character("=".into()), modifiers: Modifiers::default() });
        let _ = app.update(Message::EditChanged("=1+*2".into()));
        let _ = app.update(Message::EditSubmitted);
        app.notice = None;
        assert!(app.sheet().formula_error(cell("A1")).is_some());
        assert!(app.input_text(cell("A1")).starts_with('='));
    }

    #[test]
    fn the_theme_toggle_advances_one_preference_per_click() {
        let dir = std::env::temp_dir().join(format!("lattice-toggle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut app = Lattice::empty();
        app.folder = dir.clone();
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

        assert_eq!(app.theme_mode(), ThemeMode::Light, "system is following the desktop");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn one_click_from_system_on_a_light_desktop_pins_light() {
        let dir = std::env::temp_dir().join(format!("lattice-toggle-light-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut app = Lattice::empty();
        app.folder = dir.clone();
        app.set_system_theme(ThemeMode::Light);

        assert_eq!(app.theme_label(), "System · light");
        assert_eq!(app.theme_mode(), ThemeMode::Light);

        let _ = app.update(Message::CycleTheme);

        assert_eq!(app.theme_preference(), ThemePreference::Light);
        assert_ne!(app.theme_preference(), ThemePreference::Dark, "no step may be skipped");
        assert_eq!(app.theme_mode(), ThemeMode::Light, "system and pinned-light agree");
        assert_eq!(app.theme_label(), "Light", "the click still has to say something");

        let _ = app.update(Message::CycleTheme);
        assert_eq!(app.theme_preference(), ThemePreference::Dark);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_typed_reference_moves_the_selection_and_hides_the_box() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::NameBoxActivated);
        assert!(app.name_box.is_some(), "the box is open for typing");

        let _ = app.update(Message::NameBoxChanged("B12".into()));
        let _ = app.update(Message::NameBoxSubmitted);

        assert_eq!(app.selection(), Selection::single(gcell("B12")));
        assert!(app.name_box.is_none(), "and it goes back to a plain chip");
    }

    #[test]
    fn a_typed_reference_off_screen_is_scrolled_into_view() {
        let mut app = Lattice::empty();
        app.set_viewport(viewport());
        let _ = app.update(Message::NameBoxActivated);
        let _ = app.update(Message::NameBoxChanged("C900".into()));
        let _ = app.update(Message::NameBoxSubmitted);

        assert_eq!(app.selection(), Selection::single(gcell("C900")));
        assert!(app.scroll().y > 0.0, "the view must follow the selection");
    }

    #[test]
    fn a_typed_range_selects_the_whole_block_from_its_top_left() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::NameBoxActivated);
        let _ = app.update(Message::NameBoxChanged("D10:B2".into()));
        let _ = app.update(Message::NameBoxSubmitted);

        assert_eq!(app.selection().bounds(), gbounds("B2", "D10"));
        assert_eq!(app.selection().active, gcell("B2"), "the top-left leads the range");
        assert!(!app.selection().is_single());
    }

    #[test]
    fn a_reference_the_sheet_cannot_hold_is_refused() {
        let mut app = populated();
        app.grid.selection = Selection::single(gcell("C3"));

        for bad in ["", "nonsense", "A0", "1A", "A1048577", "XFE1", "B2:", ":D10", "B2:D10:E1"] {
            let _ = app.update(Message::NameBoxActivated);
            let _ = app.update(Message::NameBoxChanged(bad.into()));
            let _ = app.update(Message::NameBoxSubmitted);

            assert_eq!(app.selection(), Selection::single(gcell("C3")), "{bad:?} moved the cursor");
            let rejected = app.name_box.as_ref().is_some_and(|name_box| name_box.rejected);
            assert!(rejected, "{bad:?} should have been refused");
        }
    }

    #[test]
    fn escape_restores_the_name_box_without_moving_anything() {
        let mut app = Lattice::empty();
        app.grid.selection = Selection::single(gcell("D4"));

        let _ = app.update(Message::NameBoxActivated);
        let _ = app.update(Message::NameBoxChanged("ZZ99".into()));
        let _ = app.update(Message::Key { key: Key::Named(Named::Escape), modifiers: Modifiers::default() });
        assert!(app.name_box.is_none(), "Escape as a key cancels it");

        // A focused input sends Escape as a cancel
        let _ = app.update(Message::NameBoxActivated);
        let _ = app.update(Message::EditCancelled);
        assert!(app.name_box.is_none(), "Escape from the focused box cancels it too");

        assert_eq!(app.selection(), Selection::single(gcell("D4")));
    }

    #[test]
    fn a_refused_name_stops_glowing_once_the_flash_ends() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::NameBoxActivated);
        let _ = app.update(Message::NameBoxChanged("??".into()));
        let _ = app.update(Message::NameBoxSubmitted);
        assert!(app.name_box.as_ref().unwrap().rejected);

        let _ = app.update(Message::NameBoxFlashEnded);
        assert!(!app.name_box.as_ref().unwrap().rejected, "the flash is brief");

        let _ = app.update(Message::NameBoxSubmitted);
        assert!(app.name_box.as_ref().unwrap().rejected);
        let _ = app.update(Message::NameBoxChanged("A1".into()));
        assert!(!app.name_box.as_ref().unwrap().rejected, "typing clears it too");
    }

    #[test]
    fn ctrl_g_opens_the_name_box_on_the_active_cell() {
        let mut app = Lattice::empty();
        app.grid.selection = Selection::single(gcell("E6"));

        let _ = app.update(Message::Key { key: Key::Character("g".into()), modifiers: Modifiers::CTRL });

        assert_eq!(app.name_box.as_ref().map(|name_box| name_box.text.as_str()), Some("E6"));
        assert_eq!(app.selection(), Selection::single(gcell("E6")));
    }

    #[test]
    fn opening_the_name_box_commits_a_pending_cell_edit() {
        let mut app = Lattice::empty();
        let _ = app.update(Message::Key { key: Key::Character("7".into()), modifiers: Modifiers::default() });
        assert!(app.is_editing());

        let _ = app.update(Message::NameBoxActivated);

        assert!(!app.is_editing());
        assert_eq!(app.sheet().value(cell("A1")), Value::Number(7.0), "the typing was kept");
        assert_eq!(app.name_box.as_ref().map(|name_box| name_box.text.as_str()), Some("A1"));
    }
}
