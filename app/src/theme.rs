
use serde::{Deserialize, Serialize};

use iced::theme::Palette;
use iced::{color, Color, Theme};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThemeMode {
    #[default]
    Light,
    Dark,
}

impl ThemeMode {
    pub fn from_iced(mode: iced::theme::Mode) -> ThemeMode {
        match mode {
            iced::theme::Mode::Dark => ThemeMode::Dark,
            iced::theme::Mode::Light | iced::theme::Mode::None => ThemeMode::Light,
        }
    }

    pub fn palette(self) -> GardenPalette {
        match self {
            ThemeMode::Light => GardenPalette::light(),
            ThemeMode::Dark => GardenPalette::dark(),
        }
    }

    // Distinct names: iced caches styling by theme identity
    pub fn theme(self) -> Theme {
        let name = match self {
            ThemeMode::Light => "Garden Lattice",
            ThemeMode::Dark => "Night Garden",
        };
        Theme::custom(name, self.palette().iced_palette())
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemePreference {
    pub fn resolve(self, system: ThemeMode) -> ThemeMode {
        match self {
            ThemePreference::System => system,
            ThemePreference::Light => ThemeMode::Light,
            ThemePreference::Dark => ThemeMode::Dark,
        }
    }

    pub fn next(self) -> ThemePreference {
        match self {
            ThemePreference::System => ThemePreference::Light,
            ThemePreference::Light => ThemePreference::Dark,
            ThemePreference::Dark => ThemePreference::System,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemePreference::System => "System",
            ThemePreference::Light => "Light",
            ThemePreference::Dark => "Dark",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GardenPalette {
    pub canvas: Color,
    pub surface: Color,
    pub surface_deep: Color,
    pub card: Color,
    pub field: Color,

    pub lattice: Color,
    pub lattice_strong: Color,

    pub sage: Color,
    pub moss: Color,
    pub leaf: Color,
    // Accent flips: darker than leaf in light, brighter in dark
    pub leaf_bright: Color,
    pub leaf_pressed: Color,
    pub leaf_muted: Color,

    pub ink: Color,
    pub ink_soft: Color,
    pub clay: Color,
    pub warning: Color,
    // Dark text on leaf in dark mode for contrast
    pub on_leaf: Color,

    pub selection_fill: Color,
    pub selection_fill_active: Color,
    pub gutter_active: Color,
    pub button_hover: Color,
    pub button_pressed: Color,
    pub chip_wash: Color,
    pub backdrop: Color,
    pub shadow: Color,
    pub scrollbar_track: Color,
    pub scrollbar_thumb: Color,
}

impl GardenPalette {
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

    pub fn dark() -> GardenPalette {
        GardenPalette {
            canvas: color!(0x1E2420),
            surface: color!(0x252D27),
            surface_deep: color!(0x2C352E),
            card: color!(0x252D27),
            field: color!(0x333E36),

            lattice: color!(0x3D4A3B),
            lattice_strong: color!(0x4C5C49),

            sage: color!(0x7FA173),
            moss: color!(0x8FC17E),
            leaf: color!(0x6FA35F),
            leaf_bright: color!(0x8FC17E),
            leaf_pressed: color!(0x4A6B3F),
            leaf_muted: color!(0x3E4A3B),

            ink: color!(0xE8E4D9),
            ink_soft: color!(0xA8A28F),
            clay: color!(0xC17A65),
            warning: color!(0xD6A45E),
            on_leaf: color!(0x16201A),

            // Washes and shadows re-tuned: they vanish or stain on charcoal
            selection_fill: Color::from_rgba(0.50, 0.63, 0.45, 0.22),
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

pub const HAIRLINE: f32 = 1.0;
pub const FOCUS_BORDER: f32 = 1.8;

pub mod style {
    use iced::border::Radius;
    use iced::widget::container;
    use iced::widget::{button, text_input};
    use iced::{Border, Color, Shadow, Vector};

    use super::*;

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

    // A refused name burns clay instead of the focus green
    pub fn name_box(
        palette: &GardenPalette,
        rejected: bool,
        status: text_input::Status,
    ) -> text_input::Style {
        let mut style = input_style(palette, status);
        if rejected {
            style.background = palette.clay.scale_alpha(0.12).into();
            style.border = Border {
                color: palette.clay,
                width: 1.6,
                radius: 4.0.into(),
            };
        }
        style
    }

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

    // Styled from preference, not resolved mode, so a click shows
    pub fn theme_toggle(
        palette: &GardenPalette,
        preference: ThemePreference,
        status: button::Status,
    ) -> button::Style {
        let mut style = button_style(palette, status);
        let (text_color, border) = match preference {
            ThemePreference::System => (palette.ink_soft, palette.lattice),
            ThemePreference::Light | ThemePreference::Dark => (palette.leaf, palette.leaf),
        };
        style.text_color = match status {
            button::Status::Hovered | button::Status::Pressed => palette.leaf,
            _ => text_color,
        };
        style.border.color = border;
        style
    }

    pub fn modal_backdrop(palette: &GardenPalette) -> container::Style {
        container::Style {
            background: Some(palette.backdrop.into()),
            ..container::Style::default()
        }
    }

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

    #[test]
    fn the_dark_palette_is_not_a_stub() {
        let light = GardenPalette::light();
        let dark = GardenPalette::dark();
        assert_ne!(light, dark, "the dark palette is not the light one");
        for (name, colour) in [
            ("canvas", dark.canvas),
            ("ink", dark.ink),
            ("clay", dark.clay),
            ("leaf", dark.leaf),
        ] {
            assert!(colour.a > 0.0, "{name} should be drawn, not fully transparent");
        }
    }

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

            let hovered = style::theme_toggle(&palette, ThemePreference::System, button::Status::Hovered);
            assert_eq!(hovered.text_color, palette.leaf);
        }

        assert_eq!(ThemePreference::Light.label(), "Light");
        assert_eq!(ThemePreference::Dark.label(), "Dark");
    }
}
