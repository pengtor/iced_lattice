//! The one place `engine` and `lattice-grid` meet.
//!
//! The widget knows nothing about the engine: it reads a [`SheetModel`]. This
//! module adapts one to the other, and is exactly what any other host would
//! write for their own data source.

use engine::{Sheet, Value};
use lattice_grid::{Bounds, CellRef, CellValue, Dims, SheetModel};

/// The engine's sheet, seen through the widget's eyes.
#[derive(Clone, Copy)]
pub struct SheetView<'a>(pub &'a Sheet);

/// The engine's fixed spreadsheet limits, in the widget's terms.
pub fn dims() -> Dims {
    Dims {
        rows: engine::MAX_ROWS,
        cols: engine::MAX_COLS,
    }
}

// The widget owns its own CellRef/Bounds, so the two vocabularies are mapped
// here. Neither `From` impl could live in this crate: both types are foreign.

pub fn grid_cell(cell: engine::CellRef) -> CellRef {
    CellRef::new(cell.row, cell.col)
}

pub fn sheet_cell(cell: CellRef) -> engine::CellRef {
    engine::CellRef::new(cell.row, cell.col)
}

pub fn grid_bounds(bounds: engine::Bounds) -> Bounds {
    Bounds {
        min_row: bounds.min_row,
        max_row: bounds.max_row,
        min_col: bounds.min_col,
        max_col: bounds.max_col,
    }
}

/// The engine's bounds again, for the calls that only the engine can answer
/// (walking populated cells, filling a block).
pub fn sheet_bounds(bounds: Bounds) -> engine::Bounds {
    engine::Bounds {
        min_row: bounds.min_row,
        max_row: bounds.max_row,
        min_col: bounds.min_col,
        max_col: bounds.max_col,
    }
}

impl SheetModel for SheetView<'_> {
    fn dims(&self) -> Dims {
        dims()
    }

    /// The engine knows its populated extent, so Ctrl+Arrow jumps to real
    /// data instead of the sheet's edge. An empty sheet has no used range, so
    /// A1 is the honest answer.
    fn used_bounds(&self) -> Bounds {
        match self.0.used_bounds() {
            Some(bounds) => grid_bounds(bounds),
            None => Bounds::single(CellRef::new(0, 0)),
        }
    }

    fn value(&self, cell: CellRef) -> CellValue {
        match self.0.value(sheet_cell(cell)) {
            Value::Empty => CellValue::Empty,
            Value::Number(n) => CellValue::Number(n),
            Value::Text(text) => CellValue::Text(text),
            Value::Bool(flag) => CellValue::Bool(flag),
            Value::Error(kind) => CellValue::Error(kind.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The widget formats numbers for the column it paints, the engine formats
    // them for the formula bar. Same cells, so the two must agree.
    #[test]
    fn the_widget_and_the_engine_read_a_number_the_same_way() {
        let values = [
            0.1 + 0.2,
            1.0 / 3.0,
            1.0,
            -2.5,
            0.0,
            -0.0,
            1e21,
            2.5e-10,
            1.5e300,
            1234567890123456.0,
        ];
        for number in values {
            assert_eq!(
                lattice_grid::format_number(number),
                engine::format_number(number),
                "for {number}"
            );
        }
    }

    #[test]
    fn a_sheet_reads_through_the_widgets_vocabulary() {
        let sheet = Sheet::new();
        let view = SheetView(&sheet);
        assert_eq!(view.value(CellRef::new(5, 5)), CellValue::Empty);
        assert_eq!(view.dims(), dims());
        assert_eq!(view.dims().rows, engine::MAX_ROWS);
    }

    #[test]
    fn cells_and_bounds_map_between_the_two_vocabularies() {
        let cell = engine::CellRef::new(3, 4);
        assert_eq!(grid_cell(cell), CellRef::new(3, 4));
        assert_eq!(sheet_cell(grid_cell(cell)), cell);

        let bounds = grid_bounds(engine::Bounds::new(
            engine::CellRef::new(0, 0),
            engine::CellRef::new(2, 3),
        ));
        assert_eq!(bounds, Bounds::new(CellRef::new(0, 0), CellRef::new(2, 3)));
        assert!(bounds.contains(CellRef::new(2, 3)));
        assert!(!bounds.contains(CellRef::new(3, 0)));
    }
}
