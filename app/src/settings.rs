//! The app's own settings: the things that describe *you*, not a workbook.
//!
//! Kept deliberately out of the workbook format. A workbook is a document you
//! hand to someone else; the theme you prefer is a property of your installation.
//! Folding it into the `.json` would mean opening a colleague's spreadsheet
//! restyled your editor, and saving yours rewrote theirs — and a preference about
//! your own screen has no business travelling with a file of numbers.
//!
//! The settings live beside the workbooks, in the same folder the open prompt
//! reads from, so they follow the same rule: an empty folder means "here", the
//! directory the program was started in.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::{Lattice, Notice};
use crate::theme::{ThemeMode, ThemePreference};

/// The settings file's name. Deliberately not shaped like a workbook name.
pub const SETTINGS_FILE: &str = "lattice-settings.json";

/// The same name without its extension, which is what a workbook would be called
/// if someone tried to use it. That name is reserved: see
/// [`crate::persistence`], which refuses it as a workbook name and hides the
/// settings file from the open prompt's list.
pub const SETTINGS_STEM: &str = "lattice-settings";

/// Everything the app remembers about you between runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which palette to draw with, or whether to follow the desktop.
    pub theme: ThemePreference,
}

impl Settings {
    /// Where the settings live, given the workbook folder.
    pub fn path(folder: &Path) -> PathBuf {
        folder.join(SETTINGS_FILE)
    }

    /// Read the settings, falling back to defaults.
    ///
    /// This never fails, on purpose. A missing file is the ordinary first run, and
    /// an unreadable or corrupted one is not worth refusing to start over — the
    /// app opens in the system theme and writes a clean file the next time the
    /// preference changes. Losing a theme preference is a much smaller problem
    /// than a spreadsheet that will not open.
    pub fn load(folder: &Path) -> Settings {
        let Ok(text) = std::fs::read_to_string(Settings::path(folder)) else {
            return Settings::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Write the settings out, replacing any previous file.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(Settings::path(folder), text)
    }
}

impl Lattice {
    /// Adopt the preference recorded in the settings file, if there is one.
    ///
    /// Called once at startup from `boot`, not from `Lattice::new`, so that
    /// constructing state stays free of the filesystem — the tests build a
    /// `Lattice` constantly and none of them should read the developer's settings.
    pub(crate) fn load_settings(&mut self) {
        self.theme_preference = Settings::load(&self.folder).theme;
    }

    /// Note what the operating system reports.
    ///
    /// Stored unconditionally: when the preference is `System` this is what the
    /// window redraws with, and when it is an override this is what the window
    /// will go back to. Either way it is worth keeping current.
    pub(crate) fn set_system_theme(&mut self, mode: ThemeMode) {
        self.system_theme = mode;
    }

    /// Advance the toggle — System → Light → Dark → System — and remember it.
    ///
    /// The redraw needs no other prompt: the palette is read from this state on
    /// every view, so changing it here is what makes the change immediate.
    pub(crate) fn cycle_theme(&mut self) {
        self.theme_preference = self.theme_preference.next();
        self.save_settings();
    }

    /// Persist the preference, reporting a failure rather than swallowing it.
    ///
    /// A settings file that cannot be written is not fatal — the theme still
    /// changes for this session — but silently forgetting across restarts is
    /// exactly the kind of thing that reads as a bug, so it is said out loud.
    fn save_settings(&mut self) {
        let settings = Settings { theme: self.theme_preference };
        if let Err(error) = settings.save(&self.folder) {
            self.notice = Some(Notice::Problem(format!(
                "could not remember the theme preference: {error}"
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::GardenPalette;

    /// A scratch folder, so nothing is written into the repo.
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lattice-settings-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_means_follow_the_desktop() {
        let dir = scratch("missing");
        assert_eq!(
            Settings::load(&dir),
            Settings::default(),
            "a first run follows the system theme"
        );
        assert_eq!(Settings::load(&dir).theme, ThemePreference::System);
    }

    #[test]
    fn the_preference_survives_a_restart() {
        let dir = scratch("round-trip");
        let settings = Settings { theme: ThemePreference::Dark };
        settings.save(&dir).expect("the settings file is writable");

        // "Restart": read it back with nothing in memory.
        assert_eq!(Settings::load(&dir), settings);
        assert_eq!(Settings::load(&dir).theme, ThemePreference::Dark);
    }

    #[test]
    fn a_corrupt_file_does_not_stop_the_app() {
        let dir = scratch("corrupt");
        std::fs::write(Settings::path(&dir), "{ this is not json").unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
    }

    #[test]
    fn an_unknown_preference_falls_back_rather_than_failing() {
        let dir = scratch("unknown");
        std::fs::write(Settings::path(&dir), r#"{"theme":"sepia"}"#).unwrap();
        assert_eq!(Settings::load(&dir), Settings::default());
    }

    /// Adding a field later must not invalidate the file someone already has.
    #[test]
    fn unknown_fields_are_tolerated() {
        let dir = scratch("forward-compatible");
        std::fs::write(Settings::path(&dir), r#"{"theme":"dark","window":123}"#).unwrap();
        assert_eq!(Settings::load(&dir).theme, ThemePreference::Dark);
    }

    #[test]
    fn cycling_the_toggle_remembers_the_choice() {
        let dir = scratch("cycle");
        let mut app = Lattice::new();
        app.folder = dir.clone();
        assert_eq!(app.theme_preference(), ThemePreference::System);

        app.cycle_theme();
        assert_eq!(app.theme_preference(), ThemePreference::Light);
        // Written as part of the change, not at exit: there is no exit hook.
        assert_eq!(Settings::load(&dir).theme, ThemePreference::Light);

        app.cycle_theme();
        assert_eq!(Settings::load(&dir).theme, ThemePreference::Dark);

        app.cycle_theme();
        assert_eq!(Settings::load(&dir).theme, ThemePreference::System);
    }

    #[test]
    fn the_system_setting_decides_only_while_it_is_being_asked() {
        // A scratch folder, because cycling the theme writes the preference down and
        // a default folder would put it in the working directory.
        let dir = scratch("system-setting");
        let mut app = Lattice::new();
        app.folder = dir.clone();
        app.set_system_theme(ThemeMode::Dark);
        assert_eq!(app.theme_mode(), ThemeMode::Dark, "System follows the desktop");

        app.cycle_theme(); // → Light
        assert_eq!(app.theme_mode(), ThemeMode::Light, "an override wins");
        assert_eq!(app.theme_label(), "Light");

        // The desktop changing under an override must not repaint the window...
        app.set_system_theme(ThemeMode::Dark);
        assert_eq!(app.theme_mode(), ThemeMode::Light);

        app.cycle_theme(); // → Dark
        app.set_system_theme(ThemeMode::Light);
        assert_eq!(app.theme_mode(), ThemeMode::Dark, "still the override");

        // ...but switching back to System adopts whatever the desktop says *now*.
        app.cycle_theme(); // → System
        assert_eq!(app.theme_mode(), ThemeMode::Light, "back to the live setting");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_palette_follows_the_resolved_mode() {
        let mut app = Lattice::new();
        let light = app.palette();
        assert_eq!(light, GardenPalette::light());

        app.set_system_theme(ThemeMode::Dark);
        assert_eq!(app.palette(), GardenPalette::dark());
        assert_ne!(app.palette(), light, "the two modes are not one palette");
    }

    /// The settings file is not a workbook, and must not be mistaken for one: the
    /// open prompt lists every `*.json` in the folder.
    #[test]
    fn the_settings_file_is_not_shaped_like_a_workbook() {
        let dir = scratch("naming");
        let path = Settings::path(&dir);
        assert_eq!(path.file_name().unwrap(), SETTINGS_FILE);
        // The two constants have to agree, since one is used to build the path and
        // the other to reject a workbook that would land on it.
        assert_eq!(format!("{SETTINGS_STEM}.json"), SETTINGS_FILE);
        assert_ne!(
            SETTINGS_FILE, "budget.json",
            "the settings name is its own thing, not a workbook name"
        );
    }
}
