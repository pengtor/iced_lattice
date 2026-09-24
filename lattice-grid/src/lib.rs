//! A virtualised spreadsheet grid widget for iced.
//!
//! Only the visible cells are drawn, so the cost of a frame does not grow with
//! the size of the sheet. The widget is message-agnostic: it reports
//! [`GridEvent`]s from its canvas program and the host application maps them
//! into whatever message type it already speaks.
pub mod controller;
pub mod model;
pub mod sheet;
pub mod style;

pub use controller::*;
pub use model::*;
pub use sheet::*;
pub use style::{GardenPalette, FOCUS_BORDER, HAIRLINE};
