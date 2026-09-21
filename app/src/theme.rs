//! The "garden lattice" visual language, in two modes.
//!
//! The grid reads like a trellis: a calm background, soft structure lines, and a
//! small range of low-contrast greens for anything interactive. Nothing is pure
//! black or pure white, nothing is saturated, and the selection is a translucent
//! wash rather than a hard box.
//!
//! There are two palettes, [`GardenPalette::light`] (a whitewashed trellis) and
//! [`GardenPalette::dark`] ("Night Garden": the same garden after dark). They are
//! not two unrelated themes — they hold the same roles in the same relationships,
//! inverted in depth. Where light puts the *deepest* leaf on a hovered button,
//! dark puts the *brightest*; where light's card is the brightest surface on
//! screen, dark's card is the brightest surface on screen too. Reading either
//! palette on its own should tell you what the other one does.
//!
//! Every colour lives on the palette. The [`style`] functions take one explicitly
//! rather than reaching for a global, so there is exactly one place where "what
//! colour is a hovered button" is decided, per mode.

use serde::{Deserialize, Serialize};

use iced::theme::Palette;
use iced::{color, Color, Theme};

/// Which of the two palettes is in effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
}

impl ThemeMode {
    /// Read a mode out of the platform's answer.
    ///
    /// iced reports `None` when the platform does not express a preference, which
    /// is not the same as "light" — but light is the mode this app has always
    /// drawn, so it is the honest fallback rather than inventing a third look.
    pub fn from_iced(mode: iced::theme::Mode) -> ThemeMode {
        match mode {
            iced::theme::Mode::Dark => ThemeMode::Dark,
            iced::theme::Mode::Light | iced::theme::Mode::None => ThemeMode::Light,
        }
    }

    /// The palette for this mode.
    pub fn palette(self) -> GardenPalette {
        match self {
            ThemeMode::Light => GardenPalette::light(),
            ThemeMode::Dark => GardenPalette::dark(),
        }
    }

    /// The iced theme for this mode.
    ///
    /// The name differs per mode on purpose: iced caches widget styling against
    /// the theme's identity, so handing it one name for two different palettes
    /// would leave stale colours behind on a switch.
    pub fn theme(self) -> Theme {
        let name = match self {
            ThemeMode::Light => "Garden Lattice",
            ThemeMode::Dark => "Night Garden",
        };
        Theme::custom(name, self.palette().iced_palette())
    }

    /// A short word for the status line and the toggle.
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

/// What the user asked for, which is not always what they get.
///
/// `System` means "whatever the desktop says"; the other two are overrides that
/// ignore it. Kept separate from [`ThemeMode`] so that turning the override off
/// again returns to the live system setting instead of to a stale copy of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    /// Follow the operating system's light/dark setting.
    #[default]
    System,
    /// Always light, whatever the system says.
    Light,
    /// Always dark, whatever the system says.
    Dark,
}

impl ThemePreference {
    /// The mode this preference selects, given what the system currently reports.
    pub fn resolve(self, system: ThemeMode) -> ThemeMode {
        match self {
            ThemePreference::System => system,
            ThemePreference::Light => ThemeMode::Light,
            ThemePreference::Dark => ThemeMode::Dark,
        }
    }

    /// The next preference in the toggle's cycle: System → Light → Dark → System.
    pub fn next(self) -> ThemePreference {
        match self {
            ThemePreference::System => ThemePreference::Light,
            ThemePreference::Light => ThemePreference::Dark,
            ThemePreference::Dark => ThemePreference::System,
        }
    }

    /// The word on the toggle.
    pub fn label(self) -> &'static str {
        match self {
            ThemePreference::System => "System",
            ThemePreference::Light => "Light",
            ThemePreference::Dark => "Dark",
        }
    }
}

/// Every colour the app draws with, one field per role.
///
/// Deriving `Copy` is deliberate: it is small, and it lets a style closure capture
/// the palette by value without any lifetime ceremony at the call site.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GardenPalette {
    // --- surfaces, from furthest back to nearest -------------------------
    /// Cell background: the paper the sheet is drawn on.
    pub canvas: Color,
    /// Chrome, headers and gutters.
    pub surface: Color,
    /// A slightly deeper surface, for the active row/column gutter.
    pub surface_deep: Color,
    /// A modal's card: in light, the brightest sheet; in dark, the lifted one.
    pub card: Color,
    /// A focused text field.
    pub field: Color,

    // --- structure --------------------------------------------------------
    /// Structure lines, deliberately quieter than the text.
    pub lattice: Color,
    /// Slightly stronger lines, used between headers and the grid.
    pub lattice_strong: Color,

    // --- greens, quiet to loud -------------------------------------------
    /// Soft sage.
    pub sage: Color,
    /// Moss: a step deeper, for secondary structure.
    pub moss: Color,
    /// Leaf green: the primary interactive colour (active cell, fill handle, buttons).
    pub leaf: Color,
    /// The accent end of the leaf range: hover fills, and green text on the
    /// background. Darker than `leaf` in light, brighter than `leaf` in dark,
    /// because light and dark disagree about which direction is "more".
    pub leaf_bright: Color,
    /// Pressed states: a step past `leaf` away from the background.
    pub leaf_pressed: Color,
    /// A muted leaf, for disabled filled buttons.
    pub leaf_muted: Color,

    // --- text -------------------------------------------------------------
    /// Primary text: warm charcoal in light, warm off-white in dark. Never black,
    /// never pure white.
    pub ink: Color,
    /// Secondary text.
    pub ink_soft: Color,
    /// Errors: muted clay rather than fire-engine red.
    pub clay: Color,
    /// The iced `warning` role. Not used for anything specific yet, but the theme
    /// has to answer for it.
    pub warning: Color,
    /// Text drawn *on* a filled leaf button. White in light; in dark the leaf is a
    /// mid-tone, and dark text on it has roughly twice the contrast of white.
    pub on_leaf: Color,

    // --- translucent washes ----------------------------------------------
    /// A soft green wash for a selected range.
    pub selection_fill: Color,
    /// A stronger wash for the active cell within a multi-cell selection.
    pub selection_fill_active: Color,
    /// Gutter tint for the rows and columns a selection touches.
    pub gutter_active: Color,
    /// Hover wash for the outlined buttons in the top bar.
    pub button_hover: Color,
    /// Press wash for the same buttons. Denser than the hover, on both sides.
    pub button_pressed: Color,
    /// The cell-reference chip's fill.
    pub chip_wash: Color,
    /// The dimming layer behind a modal.
    pub backdrop: Color,
    /// The drop shadow under a modal card.
    pub shadow: Color,
    /// The scroll indicator's track.
    pub scrollbar_track: Color,
    /// The scroll indicator's thumb.
    pub scrollbar_thumb: Color,
}

impl GardenPalette {
    /// The palette for a mode.
    pub fn for_mode(mode: ThemeMode) -> GardenPalette {
        mode.palette()
    }

    /// The whitewashed trellis: cream paper, sage structure, soft greens.
    pub fn light() -> GardenPalette {
        GardenPalette {
            canvas: color!(0xFAF8F3),
            surface: color!(0xF5F1E8),
            surface_deep: color!(0xEFE9DC),
            card: color!(0xFAF8F3),
            field: Color::WHITE,

            lattice: color!(0xD8DFD3),
            lattice_strong: color!(0xC5CFBC),

            sage: color!(0x9BAF93),
            moss: color!(0x7D9471),
            leaf: color!(0x5C7F52),
            leaf_bright: color!(0x445C3B),
            leaf_pressed: Color::from_rgb(0.20, 0.29, 0.18),
            leaf_muted: color!(0x9BAF93),

            ink: color!(0x3E3A34),
            ink_soft: color!(0x807868),
            clay: color!(0x9C5F4E),
            warning: color!(0xB98A4B),
            on_leaf: Color::WHITE,

            selection_fill: Color::from_rgba(0.54, 0.66, 0.49, 0.18),
            selection_fill_active: Color::from_rgba(0.40, 0.55, 0.35, 0.14),
            gutter_active: Color::from_rgba(0.61, 0.69, 0.58, 0.35),
            button_hover: Color::from_rgba(0.61, 0.69, 0.58, 0.28),
            button_pressed: Color::from_rgba(0.36, 0.50, 0.32, 0.28),
            chip_wash: Color::from_rgba(0.61, 0.69, 0.58, 0.25),
            backdrop: Color::from_rgba(0.24, 0.22, 0.18, 0.30),
            shadow: Color::from_rgba(0.24, 0.22, 0.18, 0.35),
            scrollbar_track: Color::from_rgba(0.85, 0.86, 0.80, 0.18),
            scrollbar_thumb: Color::from_rgba(0.49, 0.58, 0.45, 0.55),
        }
    }

    /// Night Garden: the same garden after dark.
    ///
    /// The base is charcoal that leans green rather than grey, so the surface
    /// still belongs to the garden. The greens are lifted rather than reused: a
    /// leaf that reads as "deeper" against cream reads as "mud" against charcoal,
    /// so each one moves towards the light while keeping its neighbours' order.
    pub fn dark() -> GardenPalette {
        GardenPalette {
            canvas: color!(0x1E2420),
            surface: color!(0x252D27),
            surface_deep: color!(0x2C352E),
            // Lighter than the canvas, as the light card is lighter than its canvas.
            card: color!(0x252D27),
            field: color!(0x333E36),

            lattice: color!(0x3D4A3B),
            lattice_strong: color!(0x4C5C49),

            sage: color!(0x7FA173),
            moss: color!(0x8FC17E),
            leaf: color!(0x6FA35F),
            // The accent end flips direction here, which is the one place the two
            // palettes disagree about which way "more" points. On cream a hovered
            // button deepens, because the background is already bright; on charcoal
            // it brightens, because deepening would sink it into the background.
            leaf_bright: color!(0x8FC17E),
            leaf_pressed: color!(0x4A6B3F),
            leaf_muted: color!(0x3E4A3B),

            ink: color!(0xE8E4D9),
            ink_soft: color!(0xA8A28F),
            clay: color!(0xC17A65),
            warning: color!(0xD6A45E),
            on_leaf: color!(0x16201A),

            // Washes and shadows are re-tuned rather than reused: a 0.18 wash that
            // reads as a soft tint over cream disappears over charcoal, and a brown
            // shadow that reads as depth over cream reads as a stain over charcoal.
            selection_fill: Color::from_rgba(0.50, 0.63, 0.45, 0.22),
            // The active cell gets a *brighter* green than the wash around it, which
            // is how "more pronounced" reads when the background is dark.
            selection_fill_active: Color::from_rgba(0.56, 0.76, 0.49, 0.26),
            gutter_active: Color::from_rgba(0.50, 0.63, 0.45, 0.32),
            button_hover: Color::from_rgba(0.50, 0.63, 0.45, 0.26),
            button_pressed: Color::from_rgba(0.45, 0.63, 0.38, 0.40),
            chip_wash: Color::from_rgba(0.50, 0.63, 0.45, 0.24),
            backdrop: Color::from_rgba(0.03, 0.04, 0.03, 0.55),
            shadow: Color::from_rgba(0.00, 0.00, 0.00, 0.55),
            scrollbar_track: Color::from_rgba(0.86, 0.89, 0.83, 0.10),
            scrollbar_thumb: Color::from_rgba(0.62, 0.72, 0.57, 0.55),
        }
    }

    /// The six-role iced palette, so built-in widgets agree with the rest.
    pub fn iced_palette(&self) -> Palette {
        Palette {
            background: self.canvas,
            text: self.ink,
            primary: self.leaf,
            success: self.moss,
            warning: self.warning,
            danger: self.clay,
        }
    }
}

/// Hairline width used for grid structure.
pub const HAIRLINE: f32 = 1.0;
/// Width of the active cell outline.
pub const FOCUS_BORDER: f32 = 1.8;

/// Styles for the pieces of chrome the theme system does not reach.
///
/// Every function takes the palette it should draw from. None of them read a
/// global or a `Theme`, because a `Theme` only carries six colours and this
/// language has more roles than that.
pub mod style {
    use iced::border::Radius;
    use iced::widget::container;
    use iced::widget::{button, text_input};
    use iced::{Border, Color, Shadow, Vector};

    use super::*;

    /// The bar above the grid (title, buttons).
    pub fn top_bar(palette: &GardenPalette) -> container::Style {
        container::Style {
            background: Some(palette.surface.into()),
            text_color: Some(palette.ink),
            border: Border {
                color: palette.lattice_strong,
                width: 0.0,
                radius: Radius::default(),
            },
            ..container::Style::default()
        }
    }

    /// The formula bar strip.
    pub fn formula_bar(palette: &GardenPalette) -> container::Style {
        container::Style {
            background: Some(palette.surface.into()),
            text_color: Some(palette.ink),
            border: Border {
                color: palette.lattice,
                width: 0.0,
                radius: Radius::default(),
            },
            ..container::Style::default()
        }
    }

    /// The status strip along the bottom.
    pub fn status_bar(palette: &GardenPalette) -> container::Style {
        container::Style {
            background: Some(palette.surface.into()),
            text_color: Some(palette.ink_soft),
            border: Border {
                color: palette.lattice,
                width: 0.0,
                radius: Radius::default(),
            },
            ..container::Style::default()
        }
    }

    /// The cell reference chip (`B4`) at the left of the formula bar.
    pub fn reference_chip(palette: &GardenPalette) -> container::Style {
        container::Style {
            background: Some(palette.chip_wash.into()),
            text_color: Some(palette.leaf_bright),
            border: Border {
                color: palette.sage,
                width: 0.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }
    }

    /// Text inputs: a field raised slightly out of the bar, with a soft edge that
    /// greens on focus.
    pub fn input_style(palette: &GardenPalette, status: text_input::Status) -> text_input::Style {
        let focused = matches!(status, text_input::Status::Focused { .. });
        text_input::Style {
            background: if focused { palette.field.into() } else { palette.canvas.into() },
            border: Border {
                color: if focused { palette.leaf } else { palette.lattice_strong },
                width: if focused { 1.4 } else { HAIRLINE },
                radius: 4.0.into(),
            },
            icon: palette.ink,
            placeholder: palette.ink_soft,
            value: palette.ink,
            selection: palette.selection_fill,
        }
    }

    /// Buttons in the top bar.
    pub fn button_style(palette: &GardenPalette, status: button::Status) -> button::Style {
        let (background, text_color) = match status {
            button::Status::Active | button::Status::Disabled => {
                (Color::TRANSPARENT, palette.ink)
            }
            button::Status::Hovered => (palette.button_hover, palette.leaf),
            button::Status::Pressed => (palette.button_pressed, palette.leaf),
        };
        button::Style {
            background: Some(background.into()),
            text_color,
            border: Border { color: palette.lattice_strong, width: HAIRLINE, radius: 4.0.into() },
            ..button::Style::default()
        }
    }

    /// The confirming action in a modal (`Save`, `Open`): filled rather than outlined.
    pub fn primary_button(palette: &GardenPalette, status: button::Status) -> button::Style {
        let background = match status {
            button::Status::Active => palette.leaf,
            button::Status::Hovered => palette.leaf_bright,
            button::Status::Pressed => palette.leaf_pressed,
            button::Status::Disabled => palette.leaf_muted,
        };
        button::Style {
            background: Some(background.into()),
            text_color: palette.on_leaf,
            border: Border { color: background, width: HAIRLINE, radius: 4.0.into() },
            ..button::Style::default()
        }
    }

    /// The toggle that cycles the theme: System → Light → Dark → System.
    ///
    /// Styled from the *preference*, not from the mode it resolved to, and that is
    /// the entire point of the extra parameter. `System` is not a look of its own —
    /// it is whatever the desktop says — so on a desktop that reports light,
    /// "following the desktop" and "pinned to light" paint the whole window
    /// identically. Styled from the mode, the click between those two steps would
    /// then change nothing anywhere on screen: the preference moves, and the screen
    /// does not, which reads as a button that ignored you.
    ///
    /// So the distinction lives on the button. While the app is following the
    /// desktop the toggle stays quiet and uncommitted; once a mode has been pinned
    /// it takes the leaf accent. A click is then visible whatever the palette is
    /// doing, including on the step where the palette cannot move at all.
    pub fn theme_toggle(
        palette: &GardenPalette,
        preference: ThemePreference,
        status: button::Status,
    ) -> button::Style {
        let mut style = button_style(palette, status);
        let (text_color, border) = match preference {
            // Following: the desktop decides, so the button claims no colour of its
            // own and keeps to the faintest structure line in the palette.
            ThemePreference::System => (palette.ink_soft, palette.lattice),
            // Pinned: a choice has been made, and the button says so in the accent.
            ThemePreference::Light | ThemePreference::Dark => (palette.leaf, palette.leaf),
        };
        style.text_color = match status {
            button::Status::Hovered | button::Status::Pressed => palette.leaf,
            _ => text_color,
        };
        style.border.color = border;
        style
    }

    /// The dimming layer behind a modal: the grid stays visible but recedes.
    pub fn modal_backdrop(palette: &GardenPalette) -> container::Style {
        container::Style {
            background: Some(palette.backdrop.into()),
            ..container::Style::default()
        }
    }

    /// A modal's card: a sheet of paper floating over the dimmed grid.
    pub fn modal_card(palette: &GardenPalette) -> container::Style {
        container::Style {
            background: Some(palette.card.into()),
            text_color: Some(palette.ink),
            border: Border {
                color: palette.lattice_strong,
                width: HAIRLINE,
                radius: 8.0.into(),
            },
            shadow: Shadow {
                color: palette.shadow,
                offset: Vector::new(0.0, 6.0),
                blur_radius: 18.0,
            },
            ..container::Style::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::button;

    /// The light palette is the one that shipped before dark mode existed, and this
    /// change was not meant to alter it. Pinned role by role, so that tuning Night
    /// Garden cannot quietly repaint the day palette — the two are edited in the
    /// same file, and a stray edit is otherwise invisible until someone runs the
    /// app in light mode.
    #[test]
    fn the_light_palette_still_holds_its_original_values() {
        let p = GardenPalette::light();
        assert_eq!(p.canvas, Color::from_rgb8(0xFA, 0xF8, 0xF3));
        assert_eq!(p.surface, Color::from_rgb8(0xF5, 0xF1, 0xE8));
        assert_eq!(p.surface_deep, Color::from_rgb8(0xEF, 0xE9, 0xDC));
        assert_eq!(p.lattice, Color::from_rgb8(0xD8, 0xDF, 0xD3));
        assert_eq!(p.lattice_strong, Color::from_rgb8(0xC5, 0xCF, 0xBC));
        assert_eq!(p.sage, Color::from_rgb8(0x9B, 0xAF, 0x93));
        assert_eq!(p.moss, Color::from_rgb8(0x7D, 0x94, 0x71));
        assert_eq!(p.leaf, Color::from_rgb8(0x5C, 0x7F, 0x52));
        // Formerly `LEAF_DEEP`; it takes the accent role now.
        assert_eq!(p.leaf_bright, Color::from_rgb8(0x44, 0x5C, 0x3B));
        assert_eq!(p.leaf_muted, Color::from_rgb8(0x9B, 0xAF, 0x93));
        assert_eq!(p.ink, Color::from_rgb8(0x3E, 0x3A, 0x34));
        assert_eq!(p.ink_soft, Color::from_rgb8(0x80, 0x78, 0x68));
        assert_eq!(p.clay, Color::from_rgb8(0x9C, 0x5F, 0x4E));
        assert_eq!(p.warning, Color::from_rgb8(0xB9, 0x8A, 0x4B));
        assert_eq!(p.on_leaf, Color::WHITE);
        assert_eq!(p.field, Color::WHITE);
        assert_eq!(p.card, p.canvas);
        assert_eq!(p.selection_fill, Color::from_rgba(0.54, 0.66, 0.49, 0.18));
        assert_eq!(p.selection_fill_active, Color::from_rgba(0.40, 0.55, 0.35, 0.14));
        assert_eq!(p.gutter_active, Color::from_rgba(0.61, 0.69, 0.58, 0.35));
        assert_eq!(p.button_hover, Color::from_rgba(0.61, 0.69, 0.58, 0.28));
        assert_eq!(p.button_pressed, Color::from_rgba(0.36, 0.50, 0.32, 0.28));
        assert_eq!(p.chip_wash, Color::from_rgba(0.61, 0.69, 0.58, 0.25));
        assert_eq!(p.backdrop, Color::from_rgba(0.24, 0.22, 0.18, 0.30));
        assert_eq!(p.shadow, Color::from_rgba(0.24, 0.22, 0.18, 0.35));
        assert_eq!(p.scrollbar_track, Color::from_rgba(0.85, 0.86, 0.80, 0.18));
        assert_eq!(p.scrollbar_thumb, Color::from_rgba(0.49, 0.58, 0.45, 0.55));
    }

    /// Guards the failure the compiler cannot: two palettes that are the same, or a
    /// role left transparent so nothing draws.
    ///
    /// Note it is *not* an exhaustive check that both palettes fill every role —
    /// that one is already free, because a struct literal has to name every field,
    /// so a role added to light and forgotten in dark does not compile.
    #[test]
    fn the_dark_palette_is_not_a_stub() {
        let light = GardenPalette::light();
        let dark = GardenPalette::dark();
        assert_ne!(light, dark, "the dark palette is not the light one");
        // Every role is opaque-or-wash, but none may be left as the default that is
        // indistinguishable from "never set".
        for (name, colour) in [
            ("canvas", dark.canvas),
            ("ink", dark.ink),
            ("clay", dark.clay),
            ("leaf", dark.leaf),
        ] {
            assert!(colour.a > 0.0, "{name} should be drawn, not fully transparent");
        }
    }

    /// The dark background must actually be darker, and the text on it lighter —
    /// the whole point of the inversion.
    #[test]
    fn dark_inverts_depth() {
        let light = GardenPalette::light();
        let dark = GardenPalette::dark();

        let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;

        assert!(
            luminance(dark.canvas) < luminance(light.canvas),
            "the night canvas is darker than the day one"
        );
        assert!(
            luminance(dark.ink) > luminance(dark.canvas),
            "text is lighter than the surface it sits on"
        );
        assert!(
            luminance(light.ink) < luminance(light.canvas),
            "and the reverse in light"
        );
        assert!(
            luminance(dark.leaf) > luminance(light.leaf),
            "the leaf is brightened so it still reads against the dark ground"
        );
    }

    /// The accent end flips: darker than the leaf in light, brighter in dark.
    #[test]
    fn the_accent_end_points_away_from_the_background() {
        let light = GardenPalette::light();
        let dark = GardenPalette::dark();
        let luminance = |c: Color| 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;

        assert!(luminance(light.leaf_bright) < luminance(light.leaf));
        assert!(luminance(dark.leaf_bright) > luminance(dark.leaf));
    }

    #[test]
    fn the_toggle_cycles_back_to_the_system() {
        let mut preference = ThemePreference::System;
        let mut seen = vec![preference];
        for _ in 0..3 {
            preference = preference.next();
            seen.push(preference);
        }
        assert_eq!(
            seen,
            vec![
                ThemePreference::System,
                ThemePreference::Light,
                ThemePreference::Dark,
                ThemePreference::System,
            ]
        );
    }

    #[test]
    fn system_follows_the_desktop_and_overrides_ignore_it() {
        assert_eq!(ThemePreference::System.resolve(ThemeMode::Dark), ThemeMode::Dark);
        assert_eq!(ThemePreference::System.resolve(ThemeMode::Light), ThemeMode::Light);
        assert_eq!(ThemePreference::Light.resolve(ThemeMode::Dark), ThemeMode::Light);
        assert_eq!(ThemePreference::Dark.resolve(ThemeMode::Light), ThemeMode::Dark);
    }

    #[test]
    fn a_platform_with_no_opinion_falls_back_to_light() {
        assert_eq!(ThemeMode::from_iced(iced::theme::Mode::None), ThemeMode::Light);
        assert_eq!(ThemeMode::from_iced(iced::theme::Mode::Dark), ThemeMode::Dark);
        assert_eq!(ThemeMode::from_iced(iced::theme::Mode::Light), ThemeMode::Light);
    }

    /// A click has to be visible even on the one step where the palette cannot move.
    ///
    /// `System` and the override that agrees with the desktop resolve to the same
    /// mode, so they paint the same window: on a light desktop, "following the
    /// desktop" and "pinned to light" are the same picture, and the click between
    /// them changes no colour anywhere in the grid or the chrome. That is why the
    /// toggle is styled from the preference rather than from the resolved mode — the
    /// button itself carries the change. This test is what stops that being
    /// "simplified" back, which would make the first click from System silent again.
    #[test]
    fn the_toggle_only_looks_committed_once_a_mode_has_been_pinned() {
        for palette in [GardenPalette::light(), GardenPalette::dark()] {
            let following = style::theme_toggle(&palette, ThemePreference::System, button::Status::Active);
            let pinned = style::theme_toggle(&palette, ThemePreference::Light, button::Status::Active);
            assert_ne!(
                following.text_color, pinned.text_color,
                "clicking out of System has to show on the button"
            );
            assert_ne!(following.border.color, pinned.border.color, "and on its edge");

            // Hovering still lights up, whichever preference is showing.
            let hovered = style::theme_toggle(&palette, ThemePreference::System, button::Status::Hovered);
            assert_eq!(hovered.text_color, palette.leaf);
        }

        // The two overrides share a branch, so they are told apart by the word on the
        // button — which is why `Lattice::theme_label` names the preference, and names
        // what it resolved to as well while the desktop is deciding.
        assert_eq!(ThemePreference::Light.label(), "Light");
        assert_eq!(ThemePreference::Dark.label(), "Dark");
    }
}
