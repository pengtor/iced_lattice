//! Application state: the workbook, the selection, and the read-only surface the
//! rest of the app reads them through.
//!
//! Everything here is data plus the questions you can ask of it. Behaviour that
//! *changes* state lives in [`crate::input`] (keyboard and pointer) and
//! [`crate::persistence`] (files and the naming prompt); drawing lives in
//! [`crate::application`].
//!
//! `Lattice`'s fields are `pub(crate)` so the sibling modules can share the one
//! state value without an accessor for every pointer move.

use std::path::PathBuf;
use std::time::Instant;

use iced::keyboard::{Key, Modifiers};
use iced::{Point, Size, Vector};

use engine::sheet::Input;
use engine::{Bounds, CellRef, Sheet};

use crate::grid::Metrics;
use crate::persistence::Dialog;
use crate::theme::{GardenPalette, ThemeMode, ThemePreference};

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
pub(crate) enum Drag {
    /// Dragging out a range from the cell that was pressed.
    Selecting,
    /// Dragging the fill handle; holds the range being previewed.
    Filling(Bounds),
}

/// An in-progress edit.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Editing {
    pub(crate) text: String,
    /// The text the cell had before the edit, so that a no-op edit can be skipped.
    pub(crate) original: String,
}

/// A message shown in the status bar.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Notice {
    Info(String),
    Problem(String),
}

/// The whole application state.
pub struct Lattice {
    pub(crate) sheet: Sheet,
    pub(crate) selection: Selection,
    pub(crate) editing: Option<Editing>,
    pub(crate) scroll: Vector,
    pub(crate) viewport: Size,
    pub(crate) drag: Option<Drag>,
    pub(crate) modifiers: Modifiers,
    pub(crate) last_click: Option<(Instant, CellRef)>,
    pub(crate) notice: Option<Notice>,
    /// The name the workbook was last saved under, or `None` while it is untitled.
    pub(crate) name: Option<String>,
    /// The directory workbooks are read from and written to. Empty means "here".
    pub(crate) folder: PathBuf,
    /// The naming prompt, when one is open.
    pub(crate) dialog: Option<Dialog>,
    /// What the user asked for: follow the system, or pick a mode outright.
    pub(crate) theme_preference: ThemePreference,
    /// The last mode the operating system reported.
    ///
    /// Held separately from the resolved mode so that turning an override off
    /// again returns to what the system is saying *now*, rather than to a stale
    /// copy captured when the preference was first set.
    pub(crate) system_theme: ThemeMode,
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
            name: None,
            // Empty, so that paths display as `budget.json` rather than `./budget.json`;
            // joining onto it still resolves against the current directory.
            folder: PathBuf::new(),
            dialog: None,
            // Follow the desktop unless the settings file says otherwise. The file
            // is read separately, in `boot`, so that constructing a `Lattice` stays
            // free of I/O and predictable in tests.
            theme_preference: ThemePreference::default(),
            // Until the platform tells us otherwise, light — the mode this app has
            // always drawn.
            system_theme: ThemeMode::default(),
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

    /// The workbook's name, or `None` while it is untitled.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// What the workbook is called on screen.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("untitled spreadsheet")
    }

    /// The naming prompt, if one is open.
    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialog.as_ref()
    }

    /// The window title.
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
        Metrics::new(self.scroll, self.viewport)
    }

    // --- appearance -------------------------------------------------------

    /// What the user asked for: the system's setting, or an override.
    pub fn theme_preference(&self) -> ThemePreference {
        self.theme_preference
    }

    /// The mode actually being drawn.
    ///
    /// This is where a `System` preference becomes a concrete mode, so every
    /// caller downstream sees one of exactly two palettes and never has to ask
    /// what "system" means.
    pub fn theme_mode(&self) -> ThemeMode {
        self.theme_preference.resolve(self.system_theme)
    }

    /// Every colour the app draws with.
    pub fn palette(&self) -> GardenPalette {
        self.theme_mode().palette()
    }

    /// The iced theme, derived from the palette so the two cannot disagree.
    pub fn theme(&self) -> iced::Theme {
        self.theme_mode().theme()
    }

    /// The label on the theme toggle.
    ///
    /// Names what was asked for, and additionally what it resolved to when the
    /// two differ — "System · dark" tells you why the window just went dark,
    /// where a bare "System" would not.
    pub fn theme_label(&self) -> String {
        if self.theme_preference == ThemePreference::System {
            format!("{} · {}", self.theme_preference.label(), self.theme_mode().label())
        } else {
            self.theme_preference.label().to_string()
        }
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
    /// Save, asking for a name the first time.
    Save,
    /// Save under a different name, always asking.
    SaveAs,
    /// Open a workbook by name, always asking.
    Load,
    NewSheet,
    /// The text in the naming prompt changed.
    DialogChanged(String),
    /// The naming prompt was confirmed (button, or Enter in the field).
    DialogSubmitted,
    DialogCancelled,
    /// An existing workbook was clicked in the open prompt.
    DialogPicked(String),
    /// The theme toggle was clicked: System → Light → Dark → System.
    CycleTheme,
    /// The operating system reported its light/dark setting, at startup or
    /// whenever the desktop's theme changed underneath us.
    SystemTheme(ThemeMode),
}

#[cfg(test)]
mod tests {
    use super::test_support::{cell, populated};
    use super::*;

    #[test]
    fn a_new_workbook_is_blank() {
        let app = Lattice::new();
        assert_eq!(app.sheet().len(), 0, "a new workbook should have no cells");
        assert_eq!(app.selection().active, CellRef::new(0, 0), "cursor starts in A1");
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
    //! Helpers shared by the test modules in `state`, `input` and `persistence`,
    //! so that a workbook with something in it only has to be described once.

    use engine::{CellRef, Value};

    use super::Lattice;

    pub(crate) fn cell(a1: &str) -> CellRef {
        CellRef::parse_a1(a1).unwrap()
    }

    pub(crate) fn number(value: Value) -> f64 {
        match value {
            Value::Number(n) => n,
            other => panic!("expected a number, got {other:?}"),
        }
    }

    /// Floats are compared with a tolerance: `12 * 0.4` is `4.800000000000001` in
    /// IEEE-754 arithmetic, which is exactly the point of using `f64`.
    #[track_caller]
    pub(crate) fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "expected {expected}, got {actual}");
    }

    /// A workbook with a few rows of data, for tests that need something to
    /// aggregate, delete, fill or navigate around.
    ///
    /// The application itself now opens blank, so any test that needs contents has
    /// to bring its own — this keeps that data out of the binary.
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
