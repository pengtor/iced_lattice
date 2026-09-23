//! The grid's colours and metrics, kept with the widget so any host can restyle it.

use iced_core::theme::Palette;
use iced_core::Color;

macro_rules! color {
    ($hex:expr) => {
        Color::from_rgb8(
            (($hex >> 16) & 0xFF) as u8,
            (($hex >> 8) & 0xFF) as u8,
            ($hex & 0xFF) as u8,
        )
    };
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
    pub scrollbar_thumb_active: Color,
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
            scrollbar_track: Color::from_rgba(0.85, 0.86, 0.80, 0.40),
            scrollbar_thumb: Color::from_rgba(0.49, 0.58, 0.45, 0.80),
            scrollbar_thumb_active: Color::from_rgba(0.40, 0.52, 0.34, 0.95),
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
            scrollbar_track: Color::from_rgba(0.86, 0.89, 0.83, 0.35),
            scrollbar_thumb: Color::from_rgba(0.62, 0.72, 0.57, 0.80),
            scrollbar_thumb_active: Color::from_rgba(0.72, 0.86, 0.62, 0.95),
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
