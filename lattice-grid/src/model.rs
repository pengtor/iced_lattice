//! The vocabulary the grid speaks, and the trait a host implements to fill it.
//!
//! Nothing here knows where cell values come from. A host implements
//! [`SheetModel`] for its own data — a formula engine, a CSV reader, a database
//! cursor — and the grid paints it.

use iced_core::alignment;

/// A cell address. (0, 0) is A1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub const fn new(row: u32, col: u32) -> Self {
        CellRef { row, col }
    }
}

/// A rectangular selection, inclusive at both ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub min_row: u32,
    pub max_row: u32,
    pub min_col: u32,
    pub max_col: u32,
}

impl Bounds {
    pub fn new(a: CellRef, b: CellRef) -> Self {
        Bounds {
            min_row: a.row.min(b.row),
            max_row: a.row.max(b.row),
            min_col: a.col.min(b.col),
            max_col: a.col.max(b.col),
        }
    }

    pub fn single(cell: CellRef) -> Self {
        Bounds::new(cell, cell)
    }

    pub fn contains(&self, cell: CellRef) -> bool {
        cell.row >= self.min_row
            && cell.row <= self.max_row
            && cell.col >= self.min_col
            && cell.col <= self.max_col
    }
}

/// How many cells the sheet has. The grid never assumes a size: the host says,
/// which is what lets a 100-cell sheet have no scrollbars and a million-row one
/// scroll smoothly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dims {
    pub rows: u32,
    pub cols: u32,
}

impl Dims {
    /// The Excel limit. A spreadsheet convention, not any one host's choice.
    pub const SPREADSHEET: Dims = Dims {
        rows: 1_048_576,
        cols: 16_384,
    };
}

/// Everything the grid needs in order to paint one cell, and nothing more.
///
/// Deliberately absent: formula source, dependency edges, recalculation state,
/// number formats. Those belong to whatever produced the value.
#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    /// Already-rendered text, e.g. `#DIV/0!`.
    Error(String),
}

impl CellValue {
    pub fn is_empty(&self) -> bool {
        matches!(self, CellValue::Empty)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, CellValue::Error(_))
    }

    /// The string a cell paints when there is room for all of it.
    pub fn as_text(&self) -> String {
        match self {
            CellValue::Empty => String::new(),
            CellValue::Number(n) => format_number(*n),
            CellValue::Text(text) => text.clone(),
            CellValue::Bool(flag) => if *flag { "TRUE" } else { "FALSE" }.to_string(),
            CellValue::Error(message) => message.clone(),
        }
    }

    /// Numbers right, everything else left.
    pub fn alignment(&self) -> alignment::Horizontal {
        match self {
            CellValue::Number(_) => alignment::Horizontal::Right,
            _ => alignment::Horizontal::Left,
        }
    }
}

/// What the grid reads.
///
/// Read-only on purpose. Edits leave the widget as [`GridEvent`]s and the host
/// applies them, exactly as selection already works — so validation, undo and
/// recalculation stay on the host's side of the seam.
///
/// [`GridEvent`]: crate::GridEvent
pub trait SheetModel {
    fn dims(&self) -> Dims;

    fn value(&self, cell: CellRef) -> CellValue;
}

const DISPLAY_SIGNIFICANT_DIGITS: i32 = 15;

/// How a number reads in a cell: spreadsheet general format, 15 significant
/// digits, exponent form at the extremes.
pub fn format_number(n: f64) -> String {
    if !n.is_finite() {
        return "#NUM!".to_string();
    }
    if n == 0.0 {
        return "0".to_string(); // also normalizes -0.0
    }
    let abs = n.abs();
    if !(1e-9..1e21).contains(&abs) {
        return trim_exponent(format!("{n:.14e}"));
    }
    let rounded = if abs < 1e15 {
        round_significant(n, DISPLAY_SIGNIFICANT_DIGITS)
    } else {
        n
    };
    format!("{rounded}")
}

fn round_significant(n: f64, digits: i32) -> f64 {
    if n == 0.0 {
        return 0.0;
    }
    let exponent = n.abs().log10().floor() as i32;
    let scale = 10f64.powi(digits - 1 - exponent);
    if !scale.is_finite() {
        return n;
    }
    let scaled = n * scale;
    if !scaled.is_finite() {
        return n;
    }
    scaled.round() / scale
}

fn trim_exponent(s: String) -> String {
    let (mantissa, exponent) = match s.split_once('e') {
        Some((m, e)) => (m, e),
        None => return s,
    };
    let mantissa = if let Some(stripped) = mantissa.strip_suffix('.') {
        stripped
    } else {
        mantissa.trim_end_matches('0').trim_end_matches('.')
    };
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", exponent.strip_prefix('+').unwrap_or(exponent)),
    };
    let digits = digits.trim_start_matches('0');
    format!("{mantissa}e{sign}{}", if digits.is_empty() { "0" } else { digits })
}

/// Bijective base-26: 0->A, 25->Z, 26->AA. The column gutter's label.
pub fn col_name(mut col: u32) -> String {
    let mut buf = Vec::with_capacity(3);
    loop {
        buf.push(b'A' + (col % 26) as u8);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    buf.reverse();
    String::from_utf8(buf).expect("ascii")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_as_a_spreadsheet_writes_them() {
        assert_eq!(format_number(1.0), "1");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_number(1234567890123456.0), "1234567890123456");
        assert_eq!(format_number(1e21), "1e21");
        assert_eq!(format_number(2.5e-10), "2.5e-10");
        assert_eq!(format_number(1.5e300), "1.5e300");
        assert_eq!(format_number(f64::INFINITY), "#NUM!");
    }

    #[test]
    fn columns_are_labelled_in_bijective_base_26() {
        assert_eq!(col_name(0), "A");
        assert_eq!(col_name(1), "B");
        assert_eq!(col_name(25), "Z");
        assert_eq!(col_name(26), "AA");
        assert_eq!(col_name(27), "AB");
        assert_eq!(col_name(51), "AZ");
        assert_eq!(col_name(52), "BA");
        assert_eq!(col_name(16_383), "XFD");
    }

    #[test]
    fn numbers_are_right_aligned_and_text_is_left_aligned() {
        assert_eq!(CellValue::Number(1.0).alignment(), alignment::Horizontal::Right);
        assert_eq!(CellValue::Text("x".into()).alignment(), alignment::Horizontal::Left);
        assert_eq!(CellValue::Bool(true).alignment(), alignment::Horizontal::Left);
        assert_eq!(CellValue::Empty.alignment(), alignment::Horizontal::Left);
    }

    #[test]
    fn values_render_as_the_cell_text() {
        assert_eq!(CellValue::Empty.as_text(), "");
        assert_eq!(CellValue::Number(15.6).as_text(), "15.6");
        assert_eq!(CellValue::Text("hi".into()).as_text(), "hi");
        assert_eq!(CellValue::Bool(true).as_text(), "TRUE");
        assert_eq!(CellValue::Bool(false).as_text(), "FALSE");
        assert_eq!(CellValue::Error("#DIV/0!".into()).as_text(), "#DIV/0!");
        assert!(CellValue::Error("#DIV/0!".into()).is_error());
        assert!(CellValue::Empty.is_empty());
        assert!(!CellValue::Number(0.0).is_empty(), "a zero is not an empty cell");
    }

    #[test]
    fn bounds_normalize_however_they_are_dragged() {
        let forward = Bounds::new(CellRef::new(1, 2), CellRef::new(4, 5));
        let backward = Bounds::new(CellRef::new(4, 5), CellRef::new(1, 2));
        assert_eq!(forward, backward);
        assert_eq!(forward.min_row, 1);
        assert_eq!(forward.max_col, 5);
        assert!(forward.contains(CellRef::new(2, 3)));
        assert!(!forward.contains(CellRef::new(0, 0)));
        assert_eq!(Bounds::single(CellRef::new(7, 7)).max_row, 7);
    }
}
