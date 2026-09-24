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

    /// The cell's A1 name, e.g. `B12`. The inverse of [`CellRef::parse_a1`].
    pub fn a1(self) -> String {
        format!("{}{}", col_name(self.col), self.row + 1)
    }

    /// Reads an A1 reference, e.g. `B12`, `$B$12` or `b12`.
    ///
    /// `$` markers are accepted and dropped, the way a spreadsheet's name box
    /// treats them. Bounded by [`Dims::SPREADSHEET`], because that is what A1
    /// notation means: `A0`, `XFE1` and `A1048577` are not references to
    /// anything. A host with a smaller sheet checks its own `dims()` on top.
    pub fn parse_a1(text: &str) -> Option<CellRef> {
        let bytes = text.as_bytes();
        let mut i = 0;
        if bytes.get(i) == Some(&b'$') {
            i += 1;
        }
        let letters_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == letters_start {
            return None;
        }
        let col = col_index(&text[letters_start..i])?;
        if col >= Dims::SPREADSHEET.cols {
            return None;
        }
        if bytes.get(i) == Some(&b'$') {
            i += 1;
        }
        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i != bytes.len() || i == digits_start {
            return None;
        }
        let digits = &text[digits_start..i];
        // A leading zero is not a row number, and no row needs 8 digits
        if digits.len() > 7 || digits.starts_with('0') {
            return None;
        }
        let row: u32 = digits.parse().ok()?;
        if row == 0 || row > Dims::SPREADSHEET.rows {
            return None;
        }
        Some(CellRef::new(row - 1, col))
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

    /// Reads `B12` or `B2:D10`, normalised so the corners are sorted. The
    /// inverse of writing a range as `{min}:{max}`.
    ///
    /// Which corner ends up *active* is the host's policy, not the widget's,
    /// so this reports the block and nothing more.
    pub fn parse_a1(text: &str) -> Option<Bounds> {
        let text = text.trim();
        match text.split_once(':') {
            Some((first, second)) => {
                let first = CellRef::parse_a1(first.trim())?;
                let second = CellRef::parse_a1(second.trim())?;
                Some(Bounds::new(first, second))
            }
            None => Some(Bounds::single(CellRef::parse_a1(text)?)),
        }
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

    /// The bottom-right cell of the sheet.
    pub const fn last_cell(self) -> CellRef {
        CellRef::new(self.rows - 1, self.cols - 1)
    }
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

    /// The extent of actual data, for Ctrl+Arrow-style jump-to-edge
    /// navigation. Defaults to the full sheet, so an implementor that doesn't
    /// override this gets "jump to the sheet's edge" instead of "jump to the
    /// edge of my data" -- degraded, not broken.
    fn used_bounds(&self) -> Bounds {
        Bounds::new(CellRef::new(0, 0), self.dims().last_cell())
    }
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

/// Bijective base-26 the other way: `A`->0, `Z`->25, `AA`->26. `None` for
/// anything that is not a column label.
pub fn col_index(name: &str) -> Option<u32> {
    if name.is_empty() || name.len() > 3 {
        return None;
    }
    let mut col: u32 = 0;
    for byte in name.bytes() {
        if !byte.is_ascii_alphabetic() {
            return None;
        }
        col = col
            .checked_mul(26)?
            .checked_add(u32::from(byte.to_ascii_uppercase() - b'A') + 1)?;
    }
    Some(col - 1)
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

#[cfg(test)]
mod used_bounds_tests {
    use super::*;

    struct Nothing;

    impl SheetModel for Nothing {
        fn dims(&self) -> Dims {
            Dims { rows: 10, cols: 4 }
        }

        fn value(&self, _cell: CellRef) -> CellValue {
            CellValue::Empty
        }
    }

    #[test]
    fn a_model_that_says_nothing_about_its_data_gets_the_whole_sheet() {
        let used = Nothing.used_bounds();
        assert_eq!(used.min_row, 0);
        assert_eq!(used.min_col, 0);
        assert_eq!(used.max_row, 9, "last row of the model's own dims, not the sheet's");
        assert_eq!(used.max_col, 3);
    }

    #[test]
    fn the_sheet_extent_is_the_bottom_right_corner() {
        assert_eq!(Dims::SPREADSHEET.last_cell(), CellRef::new(1_048_575, 16_383));
        assert_eq!(Dims { rows: 1, cols: 1 }.last_cell(), CellRef::new(0, 0));
    }
}

#[cfg(test)]
mod a1_tests {
    use super::*;

    #[test]
    fn an_a1_name_round_trips_through_the_parser() {
        for (cell, name) in [
            (CellRef::new(0, 0), "A1"),
            (CellRef::new(11, 1), "B12"),
            (CellRef::new(4, 25), "Z5"),
            (CellRef::new(0, 26), "AA1"),
            (CellRef::new(1_048_575, 16_383), "XFD1048576"),
        ] {
            assert_eq!(cell.a1(), name);
            assert_eq!(CellRef::parse_a1(name), Some(cell));
        }
    }

    #[test]
    fn dollar_markers_and_case_are_accepted_and_dropped() {
        assert_eq!(CellRef::parse_a1("$B$12"), Some(CellRef::new(11, 1)));
        assert_eq!(CellRef::parse_a1("b12"), Some(CellRef::new(11, 1)));
        assert_eq!(CellRef::parse_a1("$B12"), CellRef::parse_a1("B12"));
        assert_eq!(CellRef::parse_a1("B$12"), CellRef::parse_a1("B12"));
    }

    #[test]
    fn off_sheet_text_is_not_a_reference() {
        for text in [
            "",
            " ",
            "nonsense",
            "A0",
            "1A",
            "A",
            "1",
            "A01",
            "A1048577",
            "XFE1",
            "AAAA1",
            "A1:B2",
            "A1 ",
        ] {
            assert_eq!(CellRef::parse_a1(text), None, "{text:?} parsed as a cell");
        }
    }

    #[test]
    fn a_range_parses_to_a_normalised_block() {
        assert_eq!(Bounds::parse_a1("B2:D10"), Some(Bounds::new(CellRef::new(1, 1), CellRef::new(9, 3))));
        assert_eq!(Bounds::parse_a1("D10:B2"), Bounds::parse_a1("B2:D10"), "corners sort");
        assert_eq!(Bounds::parse_a1(" C3 "), Some(Bounds::single(CellRef::new(2, 2))));
        assert_eq!(Bounds::parse_a1("B2:B2"), Some(Bounds::single(CellRef::new(1, 1))));

        for text in ["", "B2:", ":D10", "B2:D10:E1", "B2:zzz"] {
            assert_eq!(Bounds::parse_a1(text), None, "{text:?} parsed as a range");
        }
    }
}
