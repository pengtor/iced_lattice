//! Lattice — a spreadsheet with a real recalculation engine.
//!
//! This crate is only the iced front-end: it owns interaction and drawing, and
//! delegates every question about cell contents, dependencies and recalculation to
//! the `engine` crate. If the engine is the ledger, this is the desk it sits on.
//!
//! * [`application`] — state, messages, update loop, and the widget tree
//! * [`grid`] — the virtualised canvas that draws cells and reports pointer input
//! * [`theme`] — the "garden lattice" palette

pub mod application;
pub mod grid;
pub mod theme;

use application::Lattice;

/// Run the application.
pub fn run() -> iced::Result {
    iced::application(Lattice::new, Lattice::update, Lattice::view)
        .title(Lattice::title)
        .theme(garden_theme)
        .subscription(Lattice::subscription)
        .window_size((1180.0, 760.0))
        .centered()
        .run()
}

/// The one and only theme: a whitewashed trellis.
fn garden_theme(_state: &Lattice) -> iced::Theme {
    theme::garden_theme()
}
