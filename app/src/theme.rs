//! The "garden lattice" visual language.
//!
//! The grid is meant to read like a whitewashed trellis: warm cream and white
//! backgrounds, soft sage structure lines, and a small range of low-contrast greens
//! for anything interactive. Nothing is pure black, nothing is saturated, and the
//! selection is a translucent wash rather than a hard box — the intent is a surface
//! that stays calm over a long session.

use iced::theme::Palette;
use iced::{color, Color, Theme};

/// Cell background: warm white, very slightly off from paper.
pub const CANVAS: Color = color!(0xFAF8F3);
/// Chrome, headers and gutters: a touch darker, like unbleached cotton.
pub const SURFACE: Color = color!(0xF5F1E8);
/// A slightly deeper cream for the active row/column gutter.
pub const SURFACE_DEEP: Color = color!(0xEFE9DC);
/// Structure lines: pale sage, deliberately quieter than the text.
pub const LATTICE: Color = color!(0xD8DFD3);
/// Slightly stronger lines, used between headers and the grid.
pub const LATTICE_STRONG: Color = color!(0xC5CFBC);
/// Soft sage.
pub const SAGE: Color = color!(0x9BAF93);
/// Moss: a step deeper, for secondary structure.
pub const MOSS: Color = color!(0x7D9471);
/// Leaf green: the primary interactive colour (active cell, fill handle, buttons).
pub const LEAF: Color = color!(0x5C7F52);
/// A deeper leaf for pressed states and text on light green.
pub const LEAF_DEEP: Color = color!(0x445C3B);
/// Primary text: warm charcoal, never black.
pub const INK: Color = color!(0x3E3A34);
/// Secondary text: greys warmed up.
pub const INK_SOFT: Color = color!(0x807868);
/// Errors: muted clay rather than fire-engine red.
pub const CLAY: Color = color!(0x9C5F4E);

/// A soft translucent green wash for a selected range.
pub const SELECTION_FILL: Color = Color::from_rgba(0.54, 0.66, 0.49, 0.18);
/// A stronger wash for the active cell within a multi-cell selection.
pub const SELECTION_FILL_ACTIVE: Color = Color::from_rgba(0.40, 0.55, 0.35, 0.14);
/// Gutter tint for the rows and columns a selection touches.
pub const GUTTER_ACTIVE: Color = Color::from_rgba(0.61, 0.69, 0.58, 0.35);

/// Hairline width used for grid structure.
pub const HAIRLINE: f32 = 1.0;
/// Width of the active cell outline.
pub const FOCUS_BORDER: f32 = 1.8;

/// The application theme, derived from the palette above.
pub fn garden_theme() -> Theme {
    Theme::custom(
        "Garden Lattice",
        Palette {
            background: CANVAS,
            text: INK,
            primary: LEAF,
            success: MOSS,
            warning: color!(0xB98A4B),
            danger: CLAY,
        },
    )
}

/// Styles for the pieces of chrome the theme system does not reach.
pub mod style {
    use iced::border::Radius;
    use iced::widget::container;
    use iced::widget::{button, text_input};
    use iced::{Border, Color, Theme};

    use super::*;

    /// The bar above the grid (title, buttons).
    pub fn top_bar(theme: &Theme) -> container::Style {
        let _ = theme;
        container::Style {
            background: Some(SURFACE.into()),
            text_color: Some(INK),
            border: Border {
                color: LATTICE_STRONG,
                width: 0.0,
                radius: Radius::default(),
            },
            ..container::Style::default()
        }
    }

    /// The formula bar strip.
    pub fn formula_bar(theme: &Theme) -> container::Style {
        let _ = theme;
        container::Style {
            background: Some(SURFACE.into()),
            text_color: Some(INK),
            border: Border {
                color: LATTICE,
                width: 0.0,
                radius: Radius::default(),
            },
            ..container::Style::default()
        }
    }

    /// The status strip along the bottom.
    pub fn status_bar(theme: &Theme) -> container::Style {
        let _ = theme;
        container::Style {
            background: Some(SURFACE.into()),
            text_color: Some(INK_SOFT),
            border: Border {
                color: LATTICE,
                width: 0.0,
                radius: Radius::default(),
            },
            ..container::Style::default()
        }
    }

    /// The cell reference chip (`B4`) at the left of the formula bar.
    pub fn reference_chip(theme: &Theme) -> container::Style {
        let _ = theme;
        container::Style {
            background: Some(Color::from_rgba(0.61, 0.69, 0.58, 0.25).into()),
            text_color: Some(LEAF_DEEP),
            border: Border {
                color: SAGE,
                width: 0.0,
                radius: 4.0.into(),
            },
            ..container::Style::default()
        }
    }

    /// Text inputs: paper-white field with a soft sage edge that greens on focus.
    pub fn input_style(theme: &Theme, status: text_input::Status) -> text_input::Style {
        let focused = matches!(status, text_input::Status::Focused { .. });
        let palette = theme.palette();
        text_input::Style {
            background: if focused { Color::WHITE.into() } else { palette.background.into() },
            border: Border {
                color: if focused { palette.primary } else { LATTICE_STRONG },
                width: if focused { 1.4 } else { HAIRLINE },
                radius: 4.0.into(),
            },
            icon: palette.text,
            placeholder: INK_SOFT,
            value: palette.text,
            selection: SELECTION_FILL,
        }
    }

    /// Buttons in the top bar.
    pub fn button_style(theme: &Theme, status: button::Status) -> button::Style {
        let palette = theme.palette();
        let background = match status {
            button::Status::Active => Color::from_rgba(0.61, 0.69, 0.58, 0.0),
            button::Status::Hovered => Color::from_rgba(0.61, 0.69, 0.58, 0.28),
            button::Status::Pressed => Color::from_rgba(0.36, 0.50, 0.32, 0.28),
            button::Status::Disabled => Color::from_rgba(0.61, 0.69, 0.58, 0.0),
        };
        button::Style {
            background: Some(background.into()),
            text_color: match status {
                button::Status::Hovered | button::Status::Pressed => palette.primary,
                _ => palette.text,
            },
            border: Border { color: LATTICE_STRONG, width: HAIRLINE, radius: 4.0.into() },
            ..button::Style::default()
        }
    }
}
