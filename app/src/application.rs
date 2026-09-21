//! The widget tree: the top bar, formula bar, grid canvas and status bar.
//!
//! This module only draws. The data it draws comes from [`crate::state`],
//! interaction from [`crate::input`], and files and the naming prompt from
//! [`crate::persistence`].
//!
//! Every colour comes from the active [`GardenPalette`](crate::theme::GardenPalette),
//! fetched once per builder and handed to the `style` functions. The canvas is the
//! reason this is explicit rather than implicit: iced hands a `Theme` to the
//! widgets it styles, but the grid is painted by hand and receives only six
//! colours, so the palette is passed into it directly.

use iced::widget::canvas;
use iced::widget::{button, column, container, mouse_area, row, stack, text, text_input, Space};
use iced::{alignment, Element, Font, Length, Padding};

use crate::grid::{GridProgram, Metrics, CELL_WIDTH, HEADER_HEIGHT, HEADER_WIDTH};
use crate::persistence::{Dialog, Purpose};
use crate::state::{Drag, Lattice, Message, Notice};
use crate::theme;

/// Widget ids, so focus can be moved around.
pub const FORMULA_BAR: &str = "lattice-formula-bar";
pub const CELL_EDITOR: &str = "lattice-cell-editor";
pub const NAME_PROMPT: &str = "lattice-name-prompt";

impl Lattice {
    // --- view ------------------------------------------------------------

    pub fn view(&self) -> Element<'_, Message> {
        let metrics = self.metrics();
        let grid = canvas(GridProgram {
            sheet: &self.sheet,
            selection: self.selection.bounds(),
            active: self.selection.active,
            fill_preview: match self.drag {
                Some(Drag::Filling(target)) => Some(target),
                _ => None,
            },
            scroll: self.scroll,
            palette: self.palette(),
        })
        .width(Length::Fill)
        .height(Length::Fill);

        let body: Element<'_, Message> = match self.editor_overlay(&metrics) {
            Some(editor) => stack![grid, editor].into(),
            None => grid.into(),
        };

        // The naming prompt sits above everything, including an open cell editor.
        let body: Element<'_, Message> = match self.dialog.as_ref() {
            Some(dialog) => stack![body, self.dialog_overlay(dialog)].into(),
            None => body,
        };

        column![
            self.top_bar(),
            self.formula_bar(),
            body,
            self.status_bar(),
        ]
        .into()
    }

    fn top_bar(&self) -> Element<'_, Message> {
        let p = self.palette();

        // A dot marks a workbook that has never been written to disk.
        let label = if self.name.is_some() {
            self.display_name().to_string()
        } else {
            format!("{} ·", self.display_name())
        };

        let save = match &self.name {
            Some(_) => button(text("Save").size(12))
                .padding([4, 10])
                .style(move |_theme, status| theme::style::button_style(&p, status))
                .on_press(Message::Save),
            // Nothing to overwrite yet, so the button names what it is about to ask.
            None => button(text("Save…").size(12))
                .padding([4, 10])
                .style(move |_theme, status| theme::style::button_style(&p, status))
                .on_press(Message::Save),
        };

        // The theme toggle names what is in effect, so the current mode is legible
        // without opening anything: "System · dark" while following the desktop,
        // "Light" once that has been overridden.
        let preference = self.theme_preference();
        let theme_toggle = button(text(self.theme_label()).size(12))
            .padding([4, 10])
            .style(move |_theme, status| theme::style::theme_toggle(&p, preference, status))
            .on_press(Message::CycleTheme);

        container(
            row![
                text("Lattice")
                    .size(15)
                    .font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT })
                    .color(p.leaf_bright),
                text(label).size(12).color(p.ink_soft),
                Space::new().width(Length::Fill),
                button(text("New").size(12))
                    .padding([4, 10])
                    .style(move |_theme, status| theme::style::button_style(&p, status))
                    .on_press(Message::NewSheet),
                button(text("Open…").size(12))
                    .padding([4, 10])
                    .style(move |_theme, status| theme::style::button_style(&p, status))
                    .on_press(Message::Load),
                save,
                theme_toggle,
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([6, 12])
        .style(move |_theme| theme::style::top_bar(&p))
        .into()
    }

    fn formula_bar(&self) -> Element<'_, Message> {
        let p = self.palette();
        container(
            row![
                container(text(self.selection.active.a1()).size(12).color(p.leaf_bright))
                    .padding([3, 8])
                    .style(move |_theme| theme::style::reference_chip(&p)),
                text_input("value, or =formula", &self.formula_text())
                    .id(iced::widget::Id::new(FORMULA_BAR))
                    .size(13)
                    .padding(4)
                    .width(Length::Fill)
                    .style(move |_theme, status| theme::style::input_style(&p, status))
                    .on_input(Message::EditChanged)
                    .on_submit(Message::EditSubmitted),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([6, 12])
        .style(move |_theme| theme::style::formula_bar(&p))
        .into()
    }

    fn editor_overlay(&self, metrics: &Metrics) -> Option<Element<'_, Message>> {
        let editing = self.editing.as_ref()?;
        let rect = metrics.cell_rect(self.selection.active);
        let on_screen = rect.x + rect.width >= HEADER_WIDTH
            && rect.y + rect.height >= HEADER_HEIGHT
            && rect.x <= self.viewport.width
            && rect.y <= self.viewport.height;
        if !on_screen {
            return None;
        }
        let p = self.palette();

        // A text input placed exactly over the cell being edited, so the edit looks
        // like it is happening in the grid rather than in the formula bar.
        Some(
            container(
                text_input("", &editing.text)
                    .id(iced::widget::Id::new(CELL_EDITOR))
                    .size(13)
                    .padding(3)
                    .width(Length::Fixed(CELL_WIDTH + 1.0))
                    .style(move |_theme, status| theme::style::input_style(&p, status))
                    .on_input(Message::EditChanged)
                    .on_submit(Message::EditSubmitted),
            )
            .padding(Padding { top: rect.y, left: rect.x, right: 0.0, bottom: 0.0 })
            .width(Length::Fill)
            .height(Length::Fill)
            .into(),
        )
    }

    /// The naming prompt's card, centred over a dimming layer.
    ///
    /// The backdrop is a `mouse_area` that cancels on press: that both makes
    /// clicking outside the card dismiss it, and — because an interactive layer
    /// makes the stack below it transparent to pointer events — stops a stray click
    /// from reaching the grid behind the prompt.
    fn dialog_overlay(&self, dialog: &Dialog) -> Element<'_, Message> {
        let p = self.palette();

        let field = text_input("spreadsheet name", &dialog.text)
            .id(iced::widget::Id::new(NAME_PROMPT))
            .size(14)
            .padding(8)
            .width(Length::Fill)
            .style(move |_theme, status| theme::style::input_style(&p, status))
            .on_input(Message::DialogChanged)
            .on_submit(Message::DialogSubmitted);

        let mut rows = column![
            text(dialog.title())
                .size(16)
                .font(Font { weight: iced::font::Weight::Semibold, ..Font::DEFAULT })
                .color(p.leaf_bright),
            text(dialog.hint()).size(11).color(p.ink_soft),
            Space::new().height(6),
            field,
        ]
        .spacing(6)
        .width(Length::Fixed(360.0));

        if let Some(error) = &dialog.error {
            rows = rows.push(text(error.clone()).size(11).color(p.clay));
        }

        // Existing workbooks, one click away — otherwise "Open" would mean typing a
        // file name from memory.
        let existing = if dialog.purpose == Purpose::Open { self.saved_workbooks() } else { Vec::new() };
        if !existing.is_empty() {
            let mut choices = row![text("saved here:").size(11).color(p.ink_soft)].spacing(6);
            for name in existing {
                choices = choices.push(
                    button(text(name.clone()).size(11))
                        .padding([2, 8])
                        .style(move |_theme, status| theme::style::button_style(&p, status))
                        .on_press(Message::DialogPicked(name)),
                );
            }
            rows = rows.push(Space::new().height(2)).push(choices);
        }

        rows = rows
            .push(Space::new().height(4))
            .push(
                row![
                    Space::new().width(Length::Fill),
                    button(text("Cancel").size(12))
                        .padding([5, 12])
                        .style(move |_theme, status| theme::style::button_style(&p, status))
                        .on_press(Message::DialogCancelled),
                    button(text(dialog.verb()).size(12))
                        .padding([5, 14])
                        .style(move |_theme, status| theme::style::primary_button(&p, status))
                        .on_press(Message::DialogSubmitted),
                ]
                .spacing(8),
            );

        let card = container(rows)
            .padding(18)
            .style(move |_theme| theme::style::modal_card(&p));

        mouse_area(
            container(card)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(move |_theme| theme::style::modal_backdrop(&p)),
        )
        .on_press(Message::DialogCancelled)
        .into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let p = self.palette();

        let (message, color) = match &self.notice {
            Some(Notice::Problem(problem)) => (problem.clone(), p.clay),
            Some(Notice::Info(info)) => (info.clone(), p.ink_soft),
            None => {
                let active = self.selection.active;
                match self.sheet.formula_error(active) {
                    Some(diagnostic) => (diagnostic.render(&self.input_text(active)), p.clay),
                    None if !self.selection.is_single() => {
                        (format!("{} selected", self.selection.bounds().len()), p.ink_soft)
                    }
                    None => (
                        "type to edit · drag the corner to fill · Ctrl+S saves · Ctrl+Shift+S renames"
                            .to_string(),
                        p.ink_soft,
                    ),
                }
            }
        };

        // A caret diagram lines the caret up under the offending character by
        // padding with spaces, which only holds in a monospace font.
        let font = if message.contains('\n') {
            Font::MONOSPACE
        } else {
            Font::DEFAULT
        };

        container(
            row![
                text(message).size(11).font(font).color(color),
                Space::new().width(Length::Fill),
                text(format!("{} cells · {}", self.sheet.len(), self.display_name()))
                    .size(11)
                    .color(p.ink_soft),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center),
        )
        .padding([4, 12])
        .style(move |_theme| theme::style::status_bar(&p))
        .into()
    }
}
