
use std::path::PathBuf;

use iced::keyboard::{Key, Modifiers};
use iced::{Point, Size, Vector};

use engine::sheet::Input;
use engine::{CellRef, Sheet};

use crate::grid::{self, GridController, Metrics};
use crate::model;
use crate::persistence::Dialog;
use crate::theme::{GardenPalette, ThemeMode, ThemePreference};

// The interaction state and its mechanics live in the widget: selection,
// scroll, the drag in progress and double-click timing are not spreadsheet
// concerns, so `GridController` owns them and this crate drives it.
pub use crate::grid::{Drag, Selection};

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Editing {
    pub(crate) text: String,
    // Pre-edit text, so a no-op edit can be skipped
    pub(crate) original: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NameBox {
    pub(crate) text: String,
    // Set when a submission is refused, cleared by the timer
    pub(crate) rejected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    Info(String),
    Problem(String),
}

pub struct Lattice {
    pub(crate) sheet: Sheet,
    /// Selection, scroll, drag and click timing. Every grid interaction goes
    /// through this; the app only answers its hooks.
    pub(crate) grid: GridController,
    pub(crate) editing: Option<Editing>,
    pub(crate) name_box: Option<NameBox>,
    pub(crate) viewport: Size,
    pub(crate) modifiers: Modifiers,
    pub(crate) notice: Option<Notice>,
    pub(crate) name: Option<String>,
    // Empty folder means the current directory
    pub(crate) folder: PathBuf,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) theme_preference: ThemePreference,
    // Lets an override return to the live system mode
    pub(crate) system_theme: ThemeMode,
}

impl Default for Lattice {
    fn default() -> Self {
        Lattice::new()
    }
}

impl Lattice {
    pub fn new() -> Lattice {
        Lattice {
            sheet: Sheet::new(),
            grid: GridController::new(),
            editing: None,
            name_box: None,
            viewport: Size::new(1180.0, 620.0),
            modifiers: Modifiers::default(),
            notice: None,
            name: None,
            folder: PathBuf::new(),
            dialog: None,
            theme_preference: ThemePreference::default(),
            system_theme: ThemeMode::default(),
        }
    }

    pub fn empty() -> Lattice {
        Lattice::new()
    }

    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    pub fn selection(&self) -> Selection {
        self.grid.selection
    }

    pub fn scroll(&self) -> Vector {
        self.grid.scroll
    }

    pub fn viewport(&self) -> Size {
        self.viewport
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("untitled spreadsheet")
    }

    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialog.as_ref()
    }

    pub fn title(&self) -> String {
        match self.name() {
            Some(name) => format!("{name} — Lattice"),
            None => "Lattice".to_string(),
        }
    }

    pub fn set_viewport(&mut self, size: Size) {
        self.viewport = size;
    }

    pub(crate) fn metrics(&self) -> Metrics {
        Metrics::new(self.grid.scroll, self.viewport, model::dims())
    }

    pub fn theme_preference(&self) -> ThemePreference {
        self.theme_preference
    }

    pub fn theme_mode(&self) -> ThemeMode {
        self.theme_preference.resolve(self.system_theme)
    }

    pub fn palette(&self) -> GardenPalette {
        self.theme_mode().palette()
    }

    pub fn theme(&self) -> iced::Theme {
        self.theme_mode().theme()
    }

    pub fn theme_label(&self) -> String {
        if self.theme_preference == ThemePreference::System {
            format!("{} · {}", self.theme_preference.label(), self.theme_mode().label())
        } else {
            self.theme_preference.label().to_string()
        }
    }

    pub fn input_text(&self, cell: CellRef) -> String {
        match self.sheet.cell(cell).map(|stored| &stored.input) {
            Some(Input::Formula(formula)) => formula.source.clone(),
            Some(Input::Literal(value)) => value.as_text(),
            None => String::new(),
        }
    }

    pub fn formula_text(&self) -> String {
        match &self.editing {
            Some(editing) => editing.text.clone(),
            None => self.input_text(crate::model::sheet_cell(self.grid.selection.active)),
        }
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    PointerPressed { position: Point, viewport: Size },
    PointerMoved { position: Point, viewport: Size },
    PointerReleased,
    Scrolled { delta: Vector, viewport: Size },
    Viewport(Size),
    Key { key: Key, modifiers: Modifiers },
    EditChanged(String),
    EditSubmitted,
    EditCancelled,
    NameBoxActivated,
    NameBoxChanged(String),
    NameBoxSubmitted,
    NameBoxCancelled,
    NameBoxFlashEnded,
    Save,
    SaveAs,
    Load,
    NewSheet,
    DialogChanged(String),
    DialogSubmitted,
    DialogCancelled,
    DialogPicked(String),
    CycleTheme,
    SystemTheme(ThemeMode),

    // The grid's hooks. The controller owns the mechanics; these are the four
    // decisions it hands back, each answered with the app's own machinery.
    EditRequested(grid::CellRef),
    FillCommitted { source: grid::Bounds, target: grid::Bounds },
    SelectionSettled(grid::Bounds),
    ClearRequested(grid::Bounds),
}

#[cfg(test)]
mod tests {
    use super::test_support::{cell, gcell, populated};
    use super::*;

    #[test]
    fn a_new_workbook_is_blank() {
        let app = Lattice::new();
        assert_eq!(app.sheet().len(), 0, "a new workbook should have no cells");
        assert_eq!(app.selection().active, gcell("A1"), "cursor starts in A1");
        assert_eq!(app.formula_text(), "", "the formula bar starts empty");
    }

    #[test]
    fn the_formula_bar_shows_the_formula_not_the_value() {
        let app = populated();
        assert_eq!(app.input_text(cell("E2")), "=C2*D2");
        assert_eq!(app.input_text(cell("B2")), "Sage");
        assert_eq!(app.input_text(cell("Z99")), "");
    }
}

#[cfg(test)]
pub(crate) mod test_support {

    use engine::{CellRef, Value};

    use super::Lattice;

    pub(crate) fn cell(a1: &str) -> CellRef {
        CellRef::parse_a1(a1).unwrap()
    }

    // The widget's own CellRef, for the grid's selection and bounds
    pub(crate) fn gcell(a1: &str) -> crate::grid::CellRef {
        crate::grid::CellRef::parse_a1(a1).unwrap()
    }

    pub(crate) fn gbounds(first: &str, second: &str) -> crate::grid::Bounds {
        crate::grid::Bounds::new(gcell(first), gcell(second))
    }

    pub(crate) fn number(value: Value) -> f64 {
        match value {
            Value::Number(n) => n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    #[track_caller]
    pub(crate) fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "expected {expected}, got {actual}");
    }

    pub(crate) fn populated() -> Lattice {
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
}
