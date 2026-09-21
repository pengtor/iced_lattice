//! Saving and loading workbooks, and the naming prompt that names the file.
//!
//! A workbook lives on disk as JSON. There is no native file picker — iced has no
//! cross-platform dialog that would not drag in a toolkit dependency — so the
//! prompt asks for the name directly (see [`Dialog`]). Everything here touches
//! the filesystem or the prompt; no sheet arithmetic happens in this module.

use std::path::{Path, PathBuf};

use iced::advanced::widget::operation::focusable;
use iced::advanced::widget::operation::text_input as text_ops;
use iced::advanced::widget::operate;
use iced::{Task, Vector};

use engine::{CellRef, Sheet};

use crate::application::NAME_PROMPT;
use crate::state::{Lattice, Message, Notice, Selection};

/// The name offered when a workbook is saved for the first time.
const DEFAULT_NAME: &str = "Sheet1";
/// Longest name accepted, to stay well inside every filesystem's limit.
const NAME_LIMIT: usize = 64;

/// What the naming prompt is being used for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// Choose the name to write the workbook to.
    Save,
    /// Choose the name of a workbook to read back.
    Open,
}

/// The naming prompt: a spreadsheet is a file, and a file needs a name.
///
/// There is no native file picker here — iced has no cross-platform dialog that
/// would not drag in a toolkit dependency — so the prompt asks for the name
/// directly and the workbook lives beside the running program.
#[derive(Clone, Debug, PartialEq)]
pub struct Dialog {
    pub purpose: Purpose,
    pub text: String,
    /// Why the last attempt was refused, shown under the field.
    pub error: Option<String>,
}

impl Dialog {
    pub(crate) fn verb(&self) -> &'static str {
        match self.purpose {
            Purpose::Save => "Save",
            Purpose::Open => "Open",
        }
    }

    pub(crate) fn title(&self) -> &'static str {
        match self.purpose {
            Purpose::Save => "Name this spreadsheet",
            Purpose::Open => "Open a spreadsheet",
        }
    }

    pub(crate) fn hint(&self) -> String {
        match self.purpose {
            Purpose::Save => {
                "Saved as <name>.json, next to the program. Ctrl+Shift+S renames it later.".to_string()
            }
            Purpose::Open => "Reads <name>.json from the folder the program was started in.".to_string(),
        }
    }
}

impl Lattice {
    /// Where the workbook called `name` is kept.
    fn workbook_path(&self, name: &str) -> PathBuf {
        self.folder.join(format!("{name}.json"))
    }

    /// The workbooks already saved in the folder, for the open prompt to offer.
    pub(crate) fn saved_workbooks(&self) -> Vec<String> {
        // An empty `folder` means the current directory, which `read_dir` needs
        // spelled as `.` — it will not accept an empty path.
        let folder = if self.folder.as_os_str().is_empty() {
            Path::new(".")
        } else {
            self.folder.as_path()
        };
        let Ok(entries) = std::fs::read_dir(folder) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                // Only plain `.json` files directly in the folder: the prompt is
                // for picking one of ours, not for browsing.
                let is_file = entry.file_type().map(|kind| kind.is_file()).unwrap_or(false);
                if !is_file || path.extension()? != "json" {
                    return None;
                }
                // The app's own settings are also a `.json` file in this folder, but
                // they are not a workbook and must never be offered as one.
                if path.file_name().and_then(|name| name.to_str())
                    == Some(crate::settings::SETTINGS_FILE)
                {
                    return None;
                }
                Some(path.file_stem()?.to_string_lossy().into_owned())
            })
            .collect();
        names.sort();
        names.dedup();
        names.truncate(6);
        names
    }

    // --- files -----------------------------------------------------------

    /// Save the workbook, asking for a name the first time.
    ///
    /// Committing any in-progress edit first means Ctrl+S saves what is on screen,
    /// including the characters still sitting in an open editor.
    pub(crate) fn save(&mut self) -> Task<Message> {
        let settled = self.commit_edit(false);
        match self.name.clone() {
            // Already named: this is the ordinary, silent save.
            Some(name) => {
                self.write_workbook(&name);
                settled
            }
            // Never named: no file to overwrite yet, so ask before writing one.
            None => Task::batch([settled, self.open_dialog(Purpose::Save)]),
        }
    }

    /// Save As: always ask, even when the workbook already has a name.
    pub(crate) fn save_as(&mut self) -> Task<Message> {
        let settled = self.commit_edit(false);
        Task::batch([settled, self.open_dialog(Purpose::Save)])
    }

    /// Ask for a name before opening a workbook.
    pub(crate) fn open(&mut self) -> Task<Message> {
        let settled = self.commit_edit(false);
        Task::batch([settled, self.open_dialog(Purpose::Open)])
    }

    /// Write the sheet to the file called `name`, and remember the name on success.
    fn write_workbook(&mut self, name: &str) {
        let path = self.workbook_path(name);
        match engine::io::save(&self.sheet, &path, name) {
            Ok(()) => {
                self.name = Some(name.to_string());
                self.notice = Some(Notice::Info(format!(
                    "saved {} cells to {}",
                    self.sheet.len(),
                    path.display()
                )));
            }
            Err(error) => self.notice = Some(Notice::Problem(format!("save failed: {error}"))),
        }
    }

    /// Read the workbook called `name`, leaving the application untouched on failure
    /// so that the prompt can stay open and the name can be corrected.
    fn read_workbook(&mut self, name: &str) -> Result<(), engine::io::LoadError> {
        let path = self.workbook_path(name);
        let (sheet, saved_name) = engine::io::load_workbook(&path)?;
        let cells = sheet.len();

        self.sheet = sheet;
        self.selection = Selection::single(CellRef::new(0, 0));
        self.editing = None;
        self.scroll = Vector::new(0.0, 0.0);
        // The file's own name wins: it is what the workbook calls itself, and it is
        // what the next Ctrl+S should write back to.
        self.name = Some(if saved_name.is_empty() { name.to_string() } else { saved_name });
        self.notice = Some(Notice::Info(format!("opened {} ({cells} cells)", path.display())));
        Ok(())
    }

    /// Clear the workbook back to a single empty grid, leaving the file on disk
    /// untouched until the next explicit save.
    ///
    /// The name goes with it: a new workbook is untitled until it is saved.
    pub(crate) fn new_sheet(&mut self) {
        self.sheet = Sheet::new();
        self.selection = Selection::single(CellRef::new(0, 0));
        self.editing = None;
        self.scroll = Vector::new(0.0, 0.0);
        self.name = None;
        self.notice = Some(Notice::Info("new sheet".into()));
    }

    // --- the naming prompt -----------------------------------------------

    /// Open the prompt, seeded with the current name so that renaming is an edit
    /// rather than a retype.
    fn open_dialog(&mut self, purpose: Purpose) -> Task<Message> {
        self.dialog = Some(Dialog {
            purpose,
            text: self.name.clone().unwrap_or_else(|| DEFAULT_NAME.to_string()),
            error: None,
        });
        self.notice = None;
        Self::focus_dialog()
    }

    /// Focus the prompt's field with its text selected, so that typing replaces the
    /// suggested name outright.
    fn focus_dialog() -> Task<Message> {
        let id = iced::widget::Id::new(NAME_PROMPT);
        Task::batch([
            operate(focusable::focus(id.clone())),
            operate(text_ops::select_all(id)),
        ])
    }

    pub(crate) fn cancel_dialog(&mut self) -> Task<Message> {
        self.dialog = None;
        Self::unfocus()
    }

    fn set_dialog_error(&mut self, message: impl Into<String>) {
        if let Some(dialog) = &mut self.dialog {
            dialog.error = Some(message.into());
        }
    }

    /// Accept whatever is in the prompt.
    pub(crate) fn submit_dialog(&mut self) -> Task<Message> {
        let Some(dialog) = self.dialog.clone() else {
            return Task::none();
        };
        let Some(name) = Self::sanitize_name(&dialog.text) else {
            self.set_dialog_error("a spreadsheet needs a name");
            return Task::none();
        };

        match dialog.purpose {
            Purpose::Save => {
                self.dialog = None;
                self.write_workbook(&name);
                Self::unfocus()
            }
            // A name that does not exist is correctable, so the prompt stays open.
            Purpose::Open => match self.read_workbook(&name) {
                Ok(()) => {
                    self.dialog = None;
                    Self::unfocus()
                }
                Err(error) => {
                    self.set_dialog_error(error.to_string());
                    Task::none()
                }
            },
        }
    }

    /// Turn what the user typed into a usable file name.
    ///
    /// The name becomes a file name, so anything that could steer the write out of
    /// the folder (`/`, `\`, `..`) or confuse the extension is folded to a dash
    /// rather than refused: `my/budget` quietly becoming `my-budget` is friendlier
    /// than an error, and it keeps every workbook in one place.
    fn sanitize_name(raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        let stem = trimmed.strip_suffix(".json").unwrap_or(trimmed);
        let cleaned: String = stem
            .chars()
            .map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { '-' } else { c })
            .take(NAME_LIMIT)
            .collect();
        // Leading/trailing dots would make a hidden file or a `..` path segment.
        let cleaned = cleaned.trim().trim_matches('.').trim();
        // `lattice-settings.json` is the app's own file, not a workbook; refusing the
        // name stops a save from overwriting the settings that live beside it.
        if cleaned.eq_ignore_ascii_case(crate::settings::SETTINGS_STEM) {
            return None;
        }
        (!cleaned.is_empty()).then(|| cleaned.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::test_support::cell;
    use engine::Value;
    use iced::keyboard::key::Named;
    use iced::keyboard::{Key, Modifiers};

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

    /// A scratch folder for the file tests, so nothing is written into the repo.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lattice-app-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saving_asks_for_a_name_the_first_time_and_not_again() {
        let mut app = Lattice::empty();
        let dir = scratch("save");
        app.folder = dir.clone();
        app.sheet.set_input(cell("A1"), "7");

        // An untitled workbook is named before it is written anywhere.
        let _ = app.update(Message::Save);
        let dialog = app.dialog().expect("saving an untitled workbook opens the prompt");
        assert_eq!(dialog.purpose, Purpose::Save);
        assert_eq!(dialog.text, DEFAULT_NAME, "and suggests a name");
        assert!(!dir.join("Sheet1.json").exists(), "nothing is written before the name is accepted");

        // Accepting it writes the file and remembers the name.
        let _ = app.update(Message::DialogSubmitted);
        assert!(app.dialog().is_none());
        assert_eq!(app.name(), Some("Sheet1"));
        assert!(dir.join("Sheet1.json").exists());

        // With a name in hand, saving is silent — no prompt the second time.
        app.sheet.set_input(cell("A1"), "99");
        let _ = app.update(Message::Save);
        assert!(app.dialog().is_none(), "a named workbook saves straight to its own file");
        let written = std::fs::read_to_string(dir.join("Sheet1.json")).unwrap();
        assert!(written.contains("99"), "and the new value went with it");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_workbook_can_be_renamed_and_reopened_by_name() {
        let mut app = Lattice::empty();
        let dir = scratch("open");
        app.folder = dir.clone();

        app.sheet.set_input(cell("A1"), "garden");
        let _ = app.update(Message::SaveAs);
        let _ = app.update(Message::DialogChanged("planting plan".into()));
        let _ = app.update(Message::DialogSubmitted);
        assert_eq!(app.name(), Some("planting plan"));
        assert!(dir.join("planting plan.json").exists());

        // A new workbook is untitled again, and empty.
        let _ = app.update(Message::NewSheet);
        assert_eq!(app.name(), None, "a new spreadsheet has no file yet");
        assert_eq!(app.sheet().len(), 0);

        // Opening it by name restores both the cells and the name, so the next
        // Ctrl+S writes back to the file it came from.
        let _ = app.update(Message::Load);
        assert_eq!(app.dialog().unwrap().purpose, Purpose::Open);
        let _ = app.update(Message::DialogChanged("planting plan".into()));
        let _ = app.update(Message::DialogSubmitted);
        assert!(app.dialog().is_none());
        assert_eq!(app.name(), Some("planting plan"));
        assert_eq!(app.sheet().value(cell("A1")), Value::Text("garden".into()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn opening_a_missing_file_keeps_the_prompt_open_with_an_explanation() {
        let mut app = Lattice::empty();
        let dir = scratch("missing");
        app.folder = dir.clone();

        let _ = app.update(Message::Load);
        let _ = app.update(Message::DialogChanged("nowhere".into()));
        let _ = app.update(Message::DialogSubmitted);

        let dialog = app.dialog().expect("the prompt stays open so the name can be corrected");
        assert!(dialog.error.is_some(), "and says what was wrong");
        assert!(app.notice.is_none(), "the failure belongs to the prompt, not the status bar");
        assert_eq!(app.name(), None, "a failed open does not rename the workbook");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_naming_prompt_owns_the_keyboard_while_it_is_open() {
        let mut app = Lattice::empty();
        let dir = scratch("modal");
        app.folder = dir.clone();

        let _ = app.update(Message::Save);
        assert!(app.dialog().is_some());

        // Arrow keys must not move a selection the user cannot see.
        let _ = app.update(Message::Key {
            key: Key::Named(Named::ArrowDown),
            modifiers: Modifiers::default(),
        });
        assert_eq!(app.selection(), Selection::single(cell("A1")), "the grid is out of reach");

        // Escape closes the prompt without writing anything.
        let _ = app.update(Message::Key {
            key: Key::Named(Named::Escape),
            modifiers: Modifiers::default(),
        });
        assert!(app.dialog().is_none());
        assert_eq!(app.name(), None);
        assert!(!dir.join("Sheet1.json").exists(), "a cancelled save writes nothing");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_open_prompt_offers_the_workbooks_already_saved() {
        let mut app = Lattice::empty();
        let dir = scratch("listing");
        app.folder = dir.clone();
        std::fs::write(dir.join("first.json"), "{}").unwrap();
        std::fs::write(dir.join("second.json"), "{}").unwrap();
        std::fs::write(dir.join("notes.txt"), "not a workbook").unwrap();

        assert_eq!(app.saved_workbooks(), vec!["first".to_string(), "second".to_string()]);

        // Regression: the default folder is the working directory, and an empty path
        // is *not* the current directory as far as `read_dir` is concerned — it is an
        // error. The listing has to spell it `.` while the write path can stay empty.
        let default = Lattice::empty();
        assert!(default.folder.as_os_str().is_empty(), "the default folder is 'here'");
        let _ = default.saved_workbooks(); // must not panic
        assert_eq!(default.workbook_path("sheet"), PathBuf::from("sheet.json"));
        assert_eq!(default.workbook_path("sheet").display().to_string(), "sheet.json");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The settings file is a `.json` file in the same folder, and must never be
    /// offered as a workbook: "Open" would hand the parser a preferences file.
    #[test]
    fn the_settings_file_is_not_offered_as_a_workbook() {
        let mut app = Lattice::empty();
        let dir = scratch("settings-listing");
        app.folder = dir.clone();
        std::fs::write(dir.join("budget.json"), "{}").unwrap();
        std::fs::write(crate::settings::Settings::path(&dir), r#"{"theme":"dark"}"#).unwrap();

        assert_eq!(app.saved_workbooks(), vec!["budget".to_string()]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// ...and a workbook must not be allowed to *take* that name either, or saving
    /// one would overwrite the preference living beside it.
    #[test]
    fn the_settings_file_name_is_reserved() {
        assert_eq!(Lattice::sanitize_name("lattice-settings"), None);
        // Case and the extension both fold onto the same file.
        assert_eq!(Lattice::sanitize_name("Lattice-Settings.json"), None);
        // Only that name is reserved; neighbours are ordinary workbooks.
        assert_eq!(Lattice::sanitize_name("lattice-settings-2"), Some("lattice-settings-2".into()));
        assert_eq!(Lattice::sanitize_name("settings"), Some("settings".into()));
    }

    #[test]
    fn a_name_is_tidied_into_something_a_file_system_will_take() {
        assert_eq!(Lattice::sanitize_name("  budget  "), Some("budget".into()));
        assert_eq!(Lattice::sanitize_name("budget.json"), Some("budget".into()), "no doubled extension");
        assert_eq!(
            Lattice::sanitize_name("q1/q2\\q3"),
            Some("q1-q2-q3".into()),
            "separators cannot steer the write out of the folder"
        );
        assert_eq!(Lattice::sanitize_name(".."), None, "nor can a parent reference");
        assert_eq!(Lattice::sanitize_name(".hidden"), Some("hidden".into()), "no hidden files");
        assert_eq!(Lattice::sanitize_name("   "), None);
        assert!(Lattice::sanitize_name(&"x".repeat(200)).unwrap().chars().count() <= NAME_LIMIT);
    }

    #[test]
    fn saving_and_loading_round_trips_through_the_file_system() {
        let mut app = Lattice::empty();
        let dir = scratch("round-trip");
        app.folder = dir.clone();
        app.name = Some("sheet".into());

        app.sheet.set_input(cell("A1"), "7");
        let _ = app.update(Message::Save);
        assert!(matches!(app.notice, Some(Notice::Info(_))));

        app.sheet.set_input(cell("A1"), "99");
        let _ = app.update(Message::Load);
        let _ = app.update(Message::DialogSubmitted);
        assert_eq!(app.sheet().value(cell("A1")), Value::Number(7.0), "load should restore the file");
        assert_eq!(app.selection(), Selection::single(cell("A1")));

        std::fs::remove_dir_all(&dir).ok();
    }
}
