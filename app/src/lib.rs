//! Lattice — a spreadsheet with a real recalculation engine.
//!
//! This crate is only the iced front-end: it owns interaction and drawing, and
//! delegates every question about cell contents, dependencies and recalculation to
//! the `engine` crate. If the engine is the ledger, this is the desk it sits on.
//!
//! * [`state`] — the application state and the read-only surface over it
//! * [`input`] — keyboard and pointer handling, and the update loop
//! * [`persistence`] — saving, loading, and the naming prompt
//! * [`settings`] — the app's own preferences, and the file that remembers them
//! * [`application`] — the widget tree
//! * [`grid`] — the virtualised canvas that draws cells and reports pointer input
//! * [`theme`] — the "garden lattice" palettes, light and dark

use iced::Task;

pub mod application;
pub mod grid;
pub mod input;
pub mod persistence;
pub mod settings;
pub mod state;
pub mod theme;

pub use application::{CELL_EDITOR, FORMULA_BAR, NAME_PROMPT};
pub use persistence::{Dialog, Purpose};
pub use state::{Lattice, Message, Selection};
pub use theme::{GardenPalette, ThemeMode, ThemePreference};

/// Run the application.
pub fn run() -> iced::Result {
    iced::application(boot, Lattice::update, Lattice::view)
        .title(Lattice::title)
        .theme(Lattice::theme)
        .subscription(Lattice::subscription)
        .window_size((1180.0, 760.0))
        .centered()
        .run()
}

/// Open on a blank workbook, wearing the theme the user last chose.
///
/// The preference is read here rather than in `Lattice::new` so that constructing
/// state never touches the filesystem.
///
/// This also asks the platform outright which mode it is in. The subscription in
/// [`input`] reports the same thing, but it can only report it to a subscription
/// that already exists, and a window is created before that is guaranteed — so the
/// question is asked directly as well. Both routes write the same field, so
/// hearing the answer twice is harmless.
fn boot() -> (Lattice, Task<Message>) {
    let mut state = Lattice::new();
    state.load_settings();
    let system_theme = iced::system::theme()
        .map(|mode| Message::SystemTheme(ThemeMode::from_iced(mode)));
    (state, system_theme)
}
