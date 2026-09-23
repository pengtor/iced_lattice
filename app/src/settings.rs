
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::{Lattice, Notice};
use crate::theme::{ThemeMode, ThemePreference};

pub const SETTINGS_FILE: &str = "lattice-settings.json";

// Reserved: persistence refuses this as a workbook name
pub const SETTINGS_STEM: &str = "lattice-settings";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemePreference,
}

impl Settings {
    pub fn path(folder: &Path) -> PathBuf {
        folder.join(SETTINGS_FILE)
    }

    // Never fails: missing or corrupt files fall back to defaults
    pub fn load(folder: &Path) -> Settings {
        let Ok(text) = std::fs::read_to_string(Settings::path(folder)) else {
            return Settings::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(Settings::path(folder), text)
    }
}

impl Lattice {
    // Called from boot, not new(), to keep state construction I/O-free
    pub(crate) fn load_settings(&mut self) {
        self.theme_preference = Settings::load(&self.folder).theme;
    }

    pub(crate) fn set_system_theme(&mut self, mode: ThemeMode) {
        self.system_theme = mode;
    }

    pub(crate) fn cycle_theme(&mut self) {
        self.theme_preference = self.theme_preference.next();
        self.save_settings();
    }

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
        assert_eq!(Settings::load(&dir).theme, ThemePreference::Light);

        app.cycle_theme();
        assert_eq!(Settings::load(&dir).theme, ThemePreference::Dark);

        app.cycle_theme();
        assert_eq!(Settings::load(&dir).theme, ThemePreference::System);
    }

    #[test]
    fn the_system_setting_decides_only_while_it_is_being_asked() {
        let dir = scratch("system-setting");
        let mut app = Lattice::new();
        app.folder = dir.clone();
        app.set_system_theme(ThemeMode::Dark);
        assert_eq!(app.theme_mode(), ThemeMode::Dark, "System follows the desktop");

        app.cycle_theme();
        assert_eq!(app.theme_mode(), ThemeMode::Light, "an override wins");
        assert_eq!(app.theme_label(), "Light");

        app.set_system_theme(ThemeMode::Dark);
        assert_eq!(app.theme_mode(), ThemeMode::Light);

        app.cycle_theme();
        app.set_system_theme(ThemeMode::Light);
        assert_eq!(app.theme_mode(), ThemeMode::Dark, "still the override");

        app.cycle_theme();
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

    #[test]
    fn the_settings_file_is_not_shaped_like_a_workbook() {
        let dir = scratch("naming");
        let path = Settings::path(&dir);
        assert_eq!(path.file_name().unwrap(), SETTINGS_FILE);
        assert_eq!(format!("{SETTINGS_STEM}.json"), SETTINGS_FILE);
        assert_ne!(
            SETTINGS_FILE, "budget.json",
            "the settings name is its own thing, not a workbook name"
        );
    }
}
