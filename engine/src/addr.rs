//! Cell addressing: `A1`-style references, relative/absolute flags and ranges.
//!
//! Coordinates are stored 0-based internally (`CellRef { row: 0, col: 0 }` is `A1`)
//! and rendered 1-based, the way a spreadsheet user sees them.
//!
//! A [`Ref`] keeps the coordinates *as written* plus an absolute/relative flag per
//! axis. The coordinates always name the cell the reference points at right now —
//! the flags only come into play when a formula is copied or filled, at which point
//! relative components are offset (see [`Ref::shifted`]).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Number of rows in a sheet (the UI may show fewer, the engine allows this many).
pub const MAX_ROWS: u32 = 1_048_576; // 2^20, row 1048576
/// Number of columns in a sheet (`A`..`XFD`).
pub const MAX_COLS: u32 = 16_384;

/// A concrete, always-valid cell position. `(0, 0)` is `A1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellRef {
    pub row: u32,
    pub col: u32,
}

impl CellRef {
    pub const fn new(row: u32, col: u32) -> Self {
        CellRef { row, col }
    }

    /// Build a reference from 1-based row/column numbers (as typed by a user).
    pub fn from_a1_numbers(row_1based: u32, col_1based: u32) -> Option<Self> {
        if row_1based == 0 || col_1based == 0 || row_1based > MAX_ROWS || col_1based > MAX_COLS {
            return None;
        }
        Some(CellRef::new(row_1based - 1, col_1based - 1))
    }

    /// 1-based row number, as displayed in the row header.
    pub const fn row_number(self) -> u32 {
        self.row + 1
    }

    /// 1-based column number.
    pub const fn col_number(self) -> u32 {
        self.col + 1
    }

    /// Column label (`A`, `Z`, `AA`, `XFD`).
    pub fn col_name(self) -> String {
        col_name(self.col)
    }

    /// A1-style label, e.g. `B7`.
    pub fn a1(self) -> String {
        format!("{}{}", col_name(self.col), self.row + 1)
    }

    /// Parse an A1-style label (`b7`, `$B$7`, `B$7`); absolute markers are ignored.
    pub fn parse_a1(s: &str) -> Option<Self> {
        match Ref::parse(s) {
            Some(r) => r.to_cell(),
            None => None,
        }
    }

    /// Offset by a signed delta, returning `None` when the result leaves the sheet.
    pub fn checked_offset(self, row_delta: i64, col_delta: i64) -> Option<Self> {
        let row = i64::from(self.row) + row_delta;
        let col = i64::from(self.col) + col_delta;
        if row < 0 || col < 0 || row >= i64::from(MAX_ROWS) || col >= i64::from(MAX_COLS) {
            None
        } else {
            Some(CellRef::new(row as u32, col as u32))
        }
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.a1())
    }
}

/// A reference as written in a formula: coordinates plus per-axis anchoring.
///
/// `row`/`col` may be negative or past the sheet bounds when a reference was
/// produced by a fill that walked off the edge; such references evaluate to
/// `#REF!` ([`Ref::to_cell`] returns `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Ref {
    pub row: i64,
    pub col: i64,
    /// `$` before the row digits: the row does not shift on copy/fill.
    pub row_abs: bool,
    /// `$` before the column letters: the column does not shift on copy/fill.
    pub col_abs: bool,
}

impl Ref {
    /// A relative reference to `cell`.
    pub fn relative(cell: CellRef) -> Self {
        Ref { row: i64::from(cell.row), col: i64::from(cell.col), row_abs: false, col_abs: false }
    }

    /// An absolute reference to `cell` (`$A$1`).
    pub fn absolute(cell: CellRef) -> Self {
        Ref { row: i64::from(cell.row), col: i64::from(cell.col), row_abs: true, col_abs: true }
    }

    /// Parse `A1`, `$A1`, `A$1`, `$A$1` (case-insensitive).
    pub fn parse(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        let mut i = 0;
        let col_abs = bytes.get(i) == Some(&b'$');
        if col_abs {
            i += 1;
        }
        let letters_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == letters_start || i - letters_start > 3 {
            return None;
        }
        let col = col_index(&s[letters_start..i])?;
        if col >= MAX_COLS {
            return None;
        }
        let row_abs = bytes.get(i) == Some(&b'$');
        if row_abs {
            i += 1;
        }
        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i != bytes.len() || i == digits_start {
            return None;
        }
        let digits = &s[digits_start..i];
        if digits.len() > 7 || digits.starts_with('0') {
            return None;
        }
        let row_number: u32 = digits.parse().ok()?;
        if row_number == 0 || row_number > MAX_ROWS {
            return None;
        }
        Some(Ref {
            row: i64::from(row_number - 1),
            col: i64::from(col),
            row_abs,
            col_abs,
        })
    }

    /// Resolve to a concrete cell, or `None` if the reference is off-sheet.
    pub fn to_cell(self) -> Option<CellRef> {
        if self.row < 0
            || self.col < 0
            || self.row >= i64::from(MAX_ROWS)
            || self.col >= i64::from(MAX_COLS)
        {
            return None;
        }
        Some(CellRef::new(self.row as u32, self.col as u32))
    }

    /// Whether this reference is on-sheet.
    pub fn is_valid(self) -> bool {
        self.to_cell().is_some()
    }

    /// Offset relative components by the given delta (used by copy/fill).
    ///
    /// Absolute components are left alone, which is the entire point of `$`.
    pub fn shifted(self, row_delta: i64, col_delta: i64) -> Self {
        Ref {
            row: if self.row_abs { self.row } else { self.row + row_delta },
            col: if self.col_abs { self.col } else { self.col + col_delta },
            row_abs: self.row_abs,
            col_abs: self.col_abs,
        }
    }
}

impl fmt::Display for Ref {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(cell) = self.to_cell() else {
            return f.write_str("#REF!");
        };
        if self.col_abs {
            f.write_str("$")?;
        }
        f.write_str(&col_name(cell.col))?;
        if self.row_abs {
            f.write_str("$")?;
        }
        write!(f, "{}", cell.row + 1)
    }
}

/// A rectangular reference such as `A1:B10`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RangeRef {
    pub start: Ref,
    pub end: Ref,
}

/// A rectangular region of on-sheet cells, inclusive on all sides.
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

    /// A single-cell region.
    pub fn single(cell: CellRef) -> Self {
        Bounds::new(cell, cell)
    }

    pub fn contains(&self, cell: CellRef) -> bool {
        cell.row >= self.min_row
            && cell.row <= self.max_row
            && cell.col >= self.min_col
            && cell.col <= self.max_col
    }

    pub const fn rows(&self) -> u32 {
        self.max_row - self.min_row + 1
    }

    pub const fn cols(&self) -> u32 {
        self.max_col - self.min_col + 1
    }

    pub fn len(&self) -> u64 {
        u64::from(self.rows()) * u64::from(self.cols())
    }

    pub fn is_empty(&self) -> bool {
        false // a Bounds always covers at least one cell
    }

    /// Iterate row-major over every cell in the region.
    pub fn iter_cells(&self) -> impl Iterator<Item = CellRef> + '_ {
        (self.min_row..=self.max_row)
            .flat_map(move |row| (self.min_col..=self.max_col).map(move |col| CellRef::new(row, col)))
    }
}

impl RangeRef {
    pub fn new(start: CellRef, end: CellRef) -> Self {
        RangeRef { start: Ref::relative(start), end: Ref::relative(end) }
    }

    /// Resolve to an on-sheet rectangle, or `None` if either corner is off-sheet.
    pub fn bounds(self) -> Option<Bounds> {
        let a = self.start.to_cell()?;
        let b = self.end.to_cell()?;
        Some(Bounds::new(a, b))
    }

    /// Whether the range covers `cell` (false when the range itself is invalid).
    pub fn contains(self, cell: CellRef) -> bool {
        match self.bounds() {
            Some(b) => b.contains(cell),
            None => false,
        }
    }

    /// The top-left / bottom-right corners in normalized order, each keeping the
    /// anchoring it was written with.
    pub fn corners(self) -> (Ref, Ref) {
        let mut s = self.start;
        let mut e = self.end;
        if s.row > e.row {
            std::mem::swap(&mut s.row, &mut e.row);
            std::mem::swap(&mut s.row_abs, &mut e.row_abs);
        }
        if s.col > e.col {
            std::mem::swap(&mut s.col, &mut e.col);
            std::mem::swap(&mut s.col_abs, &mut e.col_abs);
        }
        (s, e)
    }

    /// Anchor each corner as it was written, e.g. `A$1:B2` keeps the `$`.
    pub fn shifted(self, row_delta: i64, col_delta: i64) -> Self {
        RangeRef { start: self.start.shifted(row_delta, col_delta), end: self.end.shifted(row_delta, col_delta) }
    }

    /// Whether either corner is off-sheet.
    pub fn is_valid(self) -> bool {
        self.bounds().is_some()
    }
}

impl fmt::Display for RangeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.start, self.end)
    }
}

/// Bijective base-26 column label: `0 -> A`, `25 -> Z`, `26 -> AA`.
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

/// Parse a column label (`a`, `XFD`); `None` if it is not a valid label.
pub fn col_index(name: &str) -> Option<u32> {
    if name.is_empty() || name.len() > 3 {
        return None;
    }
    let mut col: u32 = 0;
    for b in name.bytes() {
        if !b.is_ascii_alphabetic() {
            return None;
        }
        col = col.checked_mul(26)?.checked_add(u32::from(b.to_ascii_uppercase() - b'A') + 1)?;
    }
    Some(col - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_labels_round_trip() {
        let cases = [(0, "A"), (1, "B"), (25, "Z"), (26, "AA"), (27, "AB"), (701, "ZZ"), (702, "AAA"), (16383, "XFD")];
        for (idx, name) in cases {
            assert_eq!(col_name(idx), name);
            assert_eq!(col_index(name), Some(idx));
            assert_eq!(col_index(&name.to_ascii_lowercase()), Some(idx));
        }
    }

    #[test]
    fn parses_and_renders_a1() {
        let r = Ref::parse("B7").unwrap();
        assert_eq!(r, Ref { row: 6, col: 1, row_abs: false, col_abs: false });
        assert_eq!(r.to_string(), "B7");
        assert_eq!(r.to_cell(), Some(CellRef::new(6, 1)));
        assert_eq!(CellRef::parse_a1("$AA$10"), Some(CellRef::new(9, 26)));
    }

    #[test]
    fn preserves_absolute_markers() {
        let r = Ref::parse("$a$1").unwrap();
        assert!(r.row_abs && r.col_abs);
        assert_eq!(r.to_string(), "$A$1");
        let mixed = Ref::parse("A$1").unwrap();
        assert_eq!(mixed.to_string(), "A$1");
        assert_eq!(Ref::parse("$A1").unwrap().to_string(), "$A1");
    }

    #[test]
    fn rejects_bad_references() {
        for bad in ["", "A", "1", "A0", "A01", "1A", "AAAA1", "A1048577", "A1 ", "A1B", "#REF", "A-1", "XFE1"] {
            assert!(Ref::parse(bad).is_none(), "expected {bad:?} to be rejected");
        }
        assert!(Ref::parse("A1048576").is_some());
        assert!(Ref::parse("XFD1").is_some());
    }

    #[test]
    fn shifting_respects_anchoring() {
        let r = Ref::parse("B2").unwrap();
        assert_eq!(r.shifted(1, 1).to_string(), "C3");
        assert_eq!(Ref::parse("$B2").unwrap().shifted(1, 1).to_string(), "$B3");
        assert_eq!(Ref::parse("B$2").unwrap().shifted(1, 1).to_string(), "C$2");
        assert_eq!(Ref::parse("$B$2").unwrap().shifted(9, 9).to_string(), "$B$2");
    }

    #[test]
    fn shifting_off_sheet_is_invalid_and_renders_as_ref_error() {
        let r = Ref::parse("A1").unwrap().shifted(0, -1);
        assert!(r.to_cell().is_none());
        assert_eq!(r.to_string(), "#REF!");
        let up = Ref::parse("A1").unwrap().shifted(-1, 0);
        assert!(!up.is_valid());
    }

    #[test]
    fn ranges_normalize_and_contain() {
        let range = RangeRef { start: Ref::parse("B10").unwrap(), end: Ref::parse("A1").unwrap() };
        let bounds = range.bounds().unwrap();
        assert_eq!((bounds.min_row, bounds.min_col, bounds.max_row, bounds.max_col), (0, 0, 9, 1));
        assert!(range.contains(CellRef::new(5, 1)));
        assert!(!range.contains(CellRef::new(10, 0)));
        assert_eq!(bounds.len(), 20);
    }

    #[test]
    fn range_display_keeps_written_anchors() {
        let range = RangeRef { start: Ref::parse("A$1").unwrap(), end: Ref::parse("B2").unwrap() };
        assert_eq!(range.to_string(), "A$1:B2");
        assert_eq!(range.shifted(1, 0).to_string(), "A$1:B3");
    }
}
