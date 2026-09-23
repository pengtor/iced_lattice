
use iced::Task;

pub mod application;
pub mod model;
pub use lattice_grid as grid;
pub mod input;
pub mod persistence;
pub mod settings;
pub mod state;
pub mod theme;

pub use application::{CELL_EDITOR, FORMULA_BAR, NAME_PROMPT};
pub use persistence::{Dialog, Purpose};
pub use state::{Lattice, Message, Selection};
pub use theme::{GardenPalette, ThemeMode, ThemePreference};

pub fn run() -> iced::Result {
    iced::application(boot, Lattice::update, Lattice::view)
        .title(Lattice::title)
        .theme(Lattice::theme)
        .subscription(Lattice::subscription)
        .window_size((1180.0, 760.0))
        .centered()
        .run()
}

// Settings read here, not in Lattice::new, keeping state FS-free
// Ask platform directly too; subscription may not exist yet
fn boot() -> (Lattice, Task<Message>) {
    let mut state = Lattice::new();
    state.load_settings();
    let system_theme = iced::system::theme()
        .map(|mode| Message::SystemTheme(ThemeMode::from_iced(mode)));
    (state, system_theme)
}
