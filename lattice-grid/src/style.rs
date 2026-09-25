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

/// The grid's colours, as plain fields so a host can build its own.
///
/// [`GardenPalette::light`] and [`GardenPalette::dark`] are the two defaults.
/// Copy one and override the fields you care about if the grid should match
/// your own chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GardenPalette {
    /// The page behind everything, painted first under the whole canvas.
    pub canvas: Color,
    /// The background of the row and column gutters.
    pub surface: Color,
    /// A darker surface, used for the corner where the two gutters meet.
    pub surface_deep: Color,
    /// The background of a raised panel, such as a dialog over the grid.
    pub card: Color,
    /// The background of an input field, such as the cell editor.
    pub field: Color,

    /// The lines between cells. Drawn at a hairline width, so keep it quiet.
    pub lattice: Color,
    /// A heavier line, used for the gutter edges and for control borders.
    pub lattice_strong: Color,

    /// A muted green, for secondary accents.
    pub sage: Color,
    /// The mid green, used as the theme's success colour.
    pub moss: Color,
    /// The primary green: selection borders, the fill handle and the theme's
    /// primary colour all come from here.
    pub leaf: Color,
    // Accent flips: darker than leaf in light, brighter in dark
    /// A lighter green for hover states. It flips between the two palettes: in
    /// the light one it is darker than `leaf`, in the dark one brighter.
    pub leaf_bright: Color,
    /// The green of a pressed control.
    pub leaf_pressed: Color,
    /// The green of a disabled control.
    pub leaf_muted: Color,

    /// The main text colour, and the colour of ordinary cell values.
    pub ink: Color,
    /// A softer text colour, used for gutter labels and other secondary text.
    pub ink_soft: Color,
    /// The error colour. A cell value that doesn't compute paints in this.
    pub clay: Color,
    /// The warning colour.
    pub warning: Color,
    // Dark text on leaf in dark mode for contrast
    /// Text drawn on top of `leaf`. In the dark palette this is nearly black
    /// rather than white, so a filled green button stays readable.
    pub on_leaf: Color,

    /// The translucent wash over the cells in a selection.
    pub selection_fill: Color,
    /// A stronger wash, used for the block a fill drag is about to write and
    /// for the active cell's own gutter labels.
    pub selection_fill_active: Color,
    /// The wash over the gutter headers covered by a selection.
    pub gutter_active: Color,
    /// The background of a hovered button.
    pub button_hover: Color,
    /// The background of a pressed button.
    pub button_pressed: Color,
    /// The wash behind a chip or a tag.
    pub chip_wash: Color,
    /// The dimming layer behind a modal.
    pub backdrop: Color,
    /// The colour of drop shadows.
    pub shadow: Color,
    /// The groove a scrollbar thumb rides in.
    pub scrollbar_track: Color,
    /// A scrollbar thumb at rest.
    pub scrollbar_thumb: Color,
    /// A scrollbar thumb while it is held.
    pub scrollbar_thumb_active: Color,
}

impl GardenPalette {
    /// The light palette, and the one the grid draws with out of the box.
    ///
    /// ```
    /// use lattice_grid::GardenPalette;
    ///
    /// // Pick one to match the host application's theme.
    /// let dark_mode = true;
    /// let palette = if dark_mode { GardenPalette::dark() } else { GardenPalette::light() };
    /// ```
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

    /// The dark palette, for a host running in a dark theme.
    ///
    /// It is not the light one inverted: the washes and shadows are retuned by
    /// hand, because a wash that reads well on off-white either vanishes or
    /// stains on charcoal.
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

    /// These colours as an [`iced_core::theme::Palette`], for theming ordinary
    /// iced widgets to sit next to the grid.
    ///
    /// The mapping is fixed: `canvas` to background, `ink` to text, `leaf` to
    /// primary, `moss` to success, `warning` to warning and `clay` to danger.
    /// Anything needing a colour outside those six has to be styled by hand.
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

/// The width of a gridline, in pixels.
///
/// It is also the width used for thin borders elsewhere in the grid's chrome,
/// so a host drawing its own panels can borrow it and line up.
pub const HAIRLINE: f32 = 1.0;

/// The width of the line drawn around the selection, in pixels.
///
/// Deliberately heavier than [`HAIRLINE`], so the focus ring reads as a ring
/// and not as another gridline.
pub const FOCUS_BORDER: f32 = 1.8;
