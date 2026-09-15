//! The native file format.
//!
//! A sheet is saved as *inputs*, never as computed values: the file records what
//! each cell contains (a literal, or the text of a formula) and the engine
//! recalculates on load. That keeps the format small, readable, diff-friendly and
//! immune to the values going stale if the evaluator ever changes.
//!
//! The format is plain JSON via `serde`, versioned by [`FORMAT_VERSION`]. Unknown
//! *newer* versions are rejected rather than silently misread.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::addr::CellRef;
use crate::error::ErrorKind;
use crate::sheet::{Cell, Input, Sheet};
use crate::value::Value;

/// Version of the on-disk format written by this build.
pub const FORMAT_VERSION: u32 = 1;

/// A saved workbook.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkbookFile {
    pub version: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub cells: Vec<CellRecord>,
}

/// One cell, in a form that survives a round trip through JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellRecord {
    pub row: u32,
    pub col: u32,
    #[serde(flatten)]
    pub content: CellContent,
}

/// The contents of a cell.
///
/// Numbers are stored as JSON numbers (always finite — the engine never lets
/// `NaN`/`inf` into a cell) and formulas as their source text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellContent {
    Number { value: f64 },
    Text { value: String },
    Bool { value: bool },
    Error { value: ErrorKind },
    Formula { source: String },
}

/// Why loading failed.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("could not read file: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed workbook: {0}")]
    Format(#[from] serde_json::Error),
    #[error("workbook version {found} is newer than this build supports ({FORMAT_VERSION})")]
    UnsupportedVersion { found: u32 },
}

impl WorkbookFile {
    /// Capture a sheet as a serialisable workbook (cells in row-major order).
    pub fn from_sheet(sheet: &Sheet, name: impl Into<String>) -> WorkbookFile {
        let mut cells: Vec<CellRecord> = sheet
            .iter_cells()
            .map(|(cell, stored)| CellRecord { row: cell.row, col: cell.col, content: content_of(stored) })
            .collect();
        cells.sort_by_key(|record| (record.row, record.col));
        WorkbookFile { version: FORMAT_VERSION, name: name.into(), cells }
    }

    /// Rebuild a sheet, recalculating every formula.
    pub fn into_sheet(self) -> Result<Sheet, LoadError> {
        if self.version > FORMAT_VERSION {
            return Err(LoadError::UnsupportedVersion { found: self.version });
        }
        let mut sheet = Sheet::new();
        for record in self.cells {
            // Out-of-range coordinates are skipped rather than panicking: a
            // damaged file should not take the process down.
            if record.row >= crate::MAX_ROWS || record.col >= crate::MAX_COLS {
                continue;
            }
            let cell = CellRef::new(record.row, record.col);
            let input = match record.content {
                CellContent::Number { value } => Input::Literal(Value::Number(value)),
                CellContent::Text { value } => Input::Literal(Value::Text(value)),
                CellContent::Bool { value } => Input::Literal(Value::Bool(value)),
                CellContent::Error { value } => Input::Literal(Value::Error(value)),
                CellContent::Formula { source } => Input::Formula(crate::sheet::Formula::new(source)),
            };
            sheet.set_input_value(cell, input);
        }
        sheet.recalculate_all();
        Ok(sheet)
    }
}

fn content_of(stored: &Cell) -> CellContent {
    match &stored.input {
        Input::Formula(formula) => CellContent::Formula { source: formula.source.clone() },
        Input::Literal(value) => match value {
            Value::Empty => CellContent::Text { value: String::new() },
            Value::Number(n) => CellContent::Number { value: *n },
            Value::Text(t) => CellContent::Text { value: t.clone() },
            Value::Bool(b) => CellContent::Bool { value: *b },
            Value::Error(kind) => CellContent::Error { value: *kind },
        },
    }
}

/// Serialise a sheet to JSON.
pub fn to_string(sheet: &Sheet, name: &str) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&WorkbookFile::from_sheet(sheet, name))
}

/// Parse a sheet from JSON.
pub fn from_str(text: &str) -> Result<Sheet, LoadError> {
    Ok(from_str_workbook(text)?.0)
}

/// Parse a workbook from JSON, keeping the name it was saved under.
///
/// [`load`] throws the name away because most callers only want the data; the
/// application wants both, so that reopening a file restores its title.
pub fn from_str_workbook(text: &str) -> Result<(Sheet, String), LoadError> {
    let workbook: WorkbookFile = serde_json::from_str(text)?;
    let name = workbook.name.clone();
    Ok((workbook.into_sheet()?, name))
}

/// Write a sheet to a file.
pub fn save(sheet: &Sheet, path: impl AsRef<Path>, name: &str) -> Result<(), LoadError> {
    let json = to_string(sheet, name)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Read a sheet from a file.
pub fn load(path: impl AsRef<Path>) -> Result<Sheet, LoadError> {
    Ok(load_workbook(path)?.0)
}

/// Read a workbook from a file, keeping the name it was saved under.
pub fn load_workbook(path: impl AsRef<Path>) -> Result<(Sheet, String), LoadError> {
    let text = std::fs::read_to_string(path)?;
    from_str_workbook(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse_a1(a1).unwrap()
    }

    fn sample() -> Sheet {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "2");
        sheet.set_input(cell("A2"), "3");
        sheet.set_input(cell("A3"), "=SUM(A1:A2)");
        sheet.set_input(cell("B1"), "hello");
        sheet.set_input(cell("B2"), "TRUE");
        sheet.set_input(cell("B3"), "#N/A");
        sheet.set_input(cell("C1"), "=IF(A3>4, \"big\", \"small\")");
        sheet
    }

    #[test]
    fn round_trips_inputs_and_recomputes_values() {
        let sheet = sample();
        let json = to_string(&sheet, "Sheet1").unwrap();
        let loaded = from_str(&json).unwrap();

        assert_eq!(loaded.len(), sheet.len());
        for (cell_ref, _) in sheet.iter_cells() {
            assert_eq!(loaded.value(cell_ref), sheet.value(cell_ref), "{}", cell_ref.a1());
        }
        assert_eq!(loaded.formula_source(cell("A3")), Some("=SUM(A1:A2)"));
        // `=IF(A3>4, ...)` with A3 = 5.
        assert_eq!(loaded.value(cell("C1")), Value::Text("big".into()));
    }

    #[test]
    fn the_workbook_name_survives_a_round_trip() {
        let json = to_string(&sample(), "garden plan").unwrap();
        assert!(json.contains("\"name\": \"garden plan\""));

        let (sheet, name) = from_str_workbook(&json).unwrap();
        assert_eq!(name, "garden plan");
        assert_eq!(sheet.value(cell("A3")), Value::Number(5.0), "the cells come back too");

        // An unnamed workbook comes back with an empty name rather than failing.
        let (_, unnamed) = from_str_workbook(&to_string(&sample(), "").unwrap()).unwrap();
        assert!(unnamed.is_empty());
    }

    #[test]
    fn saved_json_is_stable_and_row_major() {
        let json = to_string(&sample(), "").unwrap();
        assert!(json.contains("\"version\": 1"));
        assert!(json.contains("\"kind\": \"formula\""));
        let first_column_zero = json.find("\"col\": 0").expect("column 0 present");
        let first_column_one = json.find("\"col\": 1").expect("column 1 present");
        assert!(first_column_zero < first_column_one, "cells should be written in row-major order");
        // Saving twice produces identical bytes.
        assert_eq!(json, to_string(&sample(), "").unwrap());
    }

    #[test]
    fn formulas_are_recalculated_on_load_rather_than_stored() {
        let json = to_string(&sample(), "").unwrap();
        assert!(!json.contains("\"value\": 5.0"), "computed values must not be stored");

        // Editing the JSON input changes the loaded value, proving values come
        // from recalculation.
        let edited = json.replace("\"value\": 3.0", "\"value\": 10.0");
        let loaded = from_str(&edited).unwrap();
        assert_eq!(loaded.value(cell("A3")), Value::Number(12.0));
    }

    #[test]
    fn rejects_newer_format_versions() {
        let json = to_string(&sample(), "").unwrap().replace("\"version\": 1", "\"version\": 99");
        match from_str(&json) {
            Err(LoadError::UnsupportedVersion { found }) => assert_eq!(found, 99),
            other => panic!("expected an unsupported-version error, got {other:?}"),
        }
    }

    #[test]
    fn reports_malformed_json_without_panicking() {
        assert!(matches!(from_str("{ not json"), Err(LoadError::Format(_))));
        assert!(from_str("{}").is_err());
    }

    #[test]
    fn skips_out_of_range_cells_instead_of_panicking() {
        let json = r#"{"version":1,"cells":[{"row":99999999,"col":0,"kind":"number","value":1}]}"#;
        let sheet = from_str(json).unwrap();
        assert!(sheet.is_empty());
    }

    #[test]
    fn saves_and_loads_through_the_filesystem() {
        let dir = std::env::temp_dir().join(format!("lattice-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sheet.json");
        save(&sample(), &path, "Sheet1").unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.value(cell("A3")), Value::Number(5.0));
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn errors_round_trip_as_error_values() {
        let sheet = sample();
        let loaded = from_str(&to_string(&sheet, "").unwrap()).unwrap();
        assert_eq!(loaded.value(cell("B3")), Value::Error(ErrorKind::NA));
    }
}
