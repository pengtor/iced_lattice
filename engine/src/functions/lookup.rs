use std::cmp::Ordering;

use super::*;

pub(super) fn dispatch(
    func: FuncId,
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
) -> Option<Value> {
    Some(match func {
        FuncId::Index => index(args, source, host),
        FuncId::Match => match_lookup(args, source, host),
        FuncId::VLookup => vector_lookup(args, source, host, Axis::Column),
        FuncId::HLookup => vector_lookup(args, source, host, Axis::Row),
        FuncId::XLookup => xlookup(args, source, host),
        _ => return None,
    })
}

// INDEX(5,1,1) must work: scalar is a one-cell array
enum Array {
    Range(Bounds),
    Scalar(Value),
}

impl Array {
    fn rows(&self) -> u32 {
        match self {
            Array::Range(bounds) => bounds.rows(),
            Array::Scalar(_) => 1,
        }
    }

    fn cols(&self) -> u32 {
        match self {
            Array::Range(bounds) => bounds.cols(),
            Array::Scalar(_) => 1,
        }
    }

    fn value_at(&self, source: &dyn ValueSource, offset: u64) -> Value {
        match self {
            Array::Range(bounds) => value_at_offset(source, *bounds, offset),
            Array::Scalar(value) => {
                if offset == 0 {
                    value.clone()
                } else {
                    Value::Empty
                }
            }
        }
    }
}

// Offsets are relative to the array's corner, not sheet coordinates
fn index(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let array = match args.get(0) {
        Some(Operand::Range(_)) => match bounds_arg(args, 0) {
            Some(bounds) => Array::Range(bounds),
            None => return Value::Error(ErrorKind::Value),
        },
        Some(Operand::Value(Value::Error(kind))) => return Value::Error(*kind),
        Some(Operand::Value(value)) => Array::Scalar(value.clone()),
        None => Array::Scalar(Value::Empty),
    };

    let row = match count_arg(args, 1, source, host) {
        Ok(row) => row,
        Err(kind) => return Value::Error(kind),
    };
    let col = match args.get(2) {
        None => None,
        Some(_) => match count_arg(args, 2, source, host) {
            Ok(col) => Some(col),
            Err(kind) => return Value::Error(kind),
        },
    };

    let rows = i64::from(array.rows());
    let cols = i64::from(array.cols());
    let (row_offset, col_offset) = match col {
        Some(col) => {
            if row < 1 || col < 1 {
                return Value::Error(ErrorKind::Value);
            }
            if row > rows || col > cols {
                return Value::Error(ErrorKind::Ref);
            }
            (row, col)
        }
        None => {
            if row < 1 {
                return Value::Error(ErrorKind::Value);
            }
            if rows == 1 {
                if row > cols {
                    return Value::Error(ErrorKind::Ref);
                }
                (1, row)
            } else if cols == 1 {
                if row > rows {
                    return Value::Error(ErrorKind::Ref);
                }
                (row, 1)
            } else {
                return Value::Error(ErrorKind::Value);
            }
        }
    };

    let offset = (row_offset - 1) as u64 * cols as u64 + (col_offset - 1) as u64;
    array.value_at(source, offset)
}

// Walk skips empties, so a blank lookup can never match
fn match_lookup(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let lookup = value_arg(args, 0, source, host);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    let bounds = match table_bounds(args, 1) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    let match_type = match opt_count_arg(args, 2, source, host, 1) {
        Ok(match_type) => match_type,
        Err(kind) => return Value::Error(kind),
    };
    if !matches!(match_type, -1 | 0 | 1) {
        return Value::Error(ErrorKind::Value);
    }

    let mut best: Option<u64> = None;
    for (offset, value) in cells_in_order(source, bounds) {
        let found = match match_type {
            0 => exact_match(&value, &lookup),
            1 => value.compare(&lookup).map(|ordering| ordering != Ordering::Greater),
            _ => value.compare(&lookup).map(|ordering| ordering != Ordering::Less),
        };
        match found {
            Ok(true) => {
                if match_type == 0 {
                    return Value::Number((offset + 1) as f64);
                }
                best = Some(offset + 1);
            }
            Ok(false) => {
                if match_type != 0 {
                    break;
                }
            }
            Err(kind) => return Value::Error(kind),
        }
    }

    match best {
        Some(position) => Value::Number(position as f64),
        None => Value::Error(ErrorKind::NA),
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Column,
    Row,
}

// approx defaults TRUE: a miss returns previous row, not #N/A
fn vector_lookup(args: &[Operand], source: &dyn ValueSource, host: CellRef, axis: Axis) -> Value {
    let lookup = value_arg(args, 0, source, host);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    let bounds = match table_bounds(args, 1) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    let index = match count_arg(args, 2, source, host) {
        Ok(index) => index,
        Err(kind) => return Value::Error(kind),
    };
    let limit = match axis {
        Axis::Column => bounds.cols(),
        Axis::Row => bounds.rows(),
    };
    if index < 1 || index > i64::from(limit) {
        return Value::Error(ErrorKind::Ref);
    }
    let approx = match opt_bool_arg(args, 3, source, host, true) {
        Ok(approx) => approx,
        Err(kind) => return Value::Error(kind),
    };

    let cols = u64::from(bounds.cols());
    let mut best: Option<u64> = None;
    for (offset, value) in cells_in_order(source, bounds) {
        if !is_header(offset, axis, cols) {
            continue;
        }
        if approx {
            match value.compare(&lookup) {
                Ok(Ordering::Greater) => break,
                Ok(_) => best = Some(offset),
                Err(kind) => return Value::Error(kind),
            }
        } else {
            match exact_match(&value, &lookup) {
                Ok(true) => {
                    let found = return_offset(offset, index, axis, cols);
                    return value_at_offset(source, bounds, found);
                }
                Ok(false) => {}
                Err(kind) => return Value::Error(kind),
            }
        }
    }

    match best {
        Some(offset) => value_at_offset(source, bounds, return_offset(offset, index, axis, cols)),
        None => Value::Error(ErrorKind::NA),
    }
}

fn is_header(offset: u64, axis: Axis, cols: u64) -> bool {
    match axis {
        Axis::Column => offset % cols == 0,
        Axis::Row => offset < cols,
    }
}

fn return_offset(header: u64, index: i64, axis: Axis, cols: u64) -> u64 {
    let index = index as u64;
    match axis {
        Axis::Column => header + (index - 1),
        Axis::Row => header + (index - 1) * cols,
    }
}

// Exact and case-insensitive, no wildcards; ranges matched position by position
fn xlookup(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let lookup = value_arg(args, 0, source, host);
    if let Value::Error(kind) = lookup {
        return Value::Error(kind);
    }
    let lookup_bounds = match table_bounds(args, 1) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    let return_bounds = match table_bounds(args, 2) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    if lookup_bounds.len() != return_bounds.len() {
        return Value::Error(ErrorKind::Value);
    }

    for (offset, value) in cells_in_order(source, lookup_bounds) {
        match value.compare(&lookup) {
            Ok(Ordering::Equal) => return value_at_offset(source, return_bounds, offset),
            Ok(_) => {}
            Err(kind) => return Value::Error(kind),
        }
    }

    if args.len() >= 4 {
        value_arg(args, 3, source, host)
    } else {
        Value::Error(ErrorKind::NA)
    }
}

fn table_bounds(args: &[Operand], i: usize) -> Result<Bounds, ErrorKind> {
    bounds_arg(args, i).ok_or(ErrorKind::Value)
}

// Text lookups are wildcard patterns (e.g. "b*" prefix match)
fn exact_match(cell: &Value, lookup: &Value) -> Result<bool, ErrorKind> {
    match lookup {
        Value::Text(pattern) => match cell {
            Value::Text(text) => Ok(wildcard_match(pattern, text)),
            _ => Ok(false),
        },
        other => Ok(other.compare(cell)? == Ordering::Equal),
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{assert_eq_value, eval_at};
    use super::*;

    #[track_caller]
    fn assert_eq_value_at(src: &str, cells: &[(&str, Value)], expected: Value) {
        assert_eq!(eval_at(src, cells, "E1"), expected, "{src}");
    }

    #[test]
    fn index_reads_a_cell_by_relative_offset() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("B1", Value::Number(2.0)),
            ("C1", Value::Number(3.0)),
            ("A2", Value::Number(4.0)),
            ("B2", Value::Number(5.0)),
            ("C2", Value::Number(6.0)),
        ];
        assert_eq_value_at("=INDEX(A1:C2, 2, 3)", &cells, Value::Number(6.0));
        assert_eq_value_at("=INDEX(A1:C2, 1, 1)", &cells, Value::Number(1.0));
    }

    #[test]
    fn index_uses_the_single_index_along_the_only_axis() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("B1", Value::Number(2.0)),
            ("C1", Value::Number(3.0)),
            ("A2", Value::Number(4.0)),
            ("A3", Value::Number(5.0)),
        ];
        assert_eq_value_at("=INDEX(A1:C1, 3)", &cells, Value::Number(3.0));
        assert_eq_value_at("=INDEX(A1:A3, 3)", &cells, Value::Number(5.0));
        assert_eq_value_at("=INDEX(A1:C2, 2)", &cells, Value::Error(ErrorKind::Value));
    }

    #[test]
    fn index_accepts_a_scalar_as_a_one_cell_array() {
        assert_eq_value("=INDEX(5, 1, 1)", Value::Number(5.0));
        assert_eq_value("=INDEX(\"x\", 1, 1)", Value::Text("x".into()));
        assert_eq_value("=INDEX(5, 1, 2)", Value::Error(ErrorKind::Ref));
    }

    #[test]
    fn index_rejects_zero_and_out_of_range_offsets() {
        let cells = [("A1", Value::Number(1.0)), ("B1", Value::Number(2.0))];
        assert_eq_value_at("=INDEX(A1:B1, 0, 1)", &cells, Value::Error(ErrorKind::Value));
        assert_eq_value_at("=INDEX(A1:B1, 1, 0)", &cells, Value::Error(ErrorKind::Value));
        assert_eq_value_at("=INDEX(A1:B1, 3, 1)", &cells, Value::Error(ErrorKind::Ref));
    }

    #[test]
    fn match_finds_exact_and_approximate_positions() {
        let ascending = [
            ("A1", Value::Number(10.0)),
            ("A2", Value::Number(20.0)),
            ("A3", Value::Number(30.0)),
            ("A4", Value::Number(40.0)),
            ("A5", Value::Number(50.0)),
        ];
        assert_eq_value_at("=MATCH(30, A1:A5, 0)", &ascending, Value::Number(3.0));
        assert_eq_value_at("=MATCH(35, A1:A5)", &ascending, Value::Number(3.0));
        assert_eq_value_at("=MATCH(35, A1:A5, 1)", &ascending, Value::Number(3.0));
        assert_eq_value_at("=MATCH(50, A1:A5)", &ascending, Value::Number(5.0));
        assert_eq_value_at("=MATCH(5, A1:A5)", &ascending, Value::Error(ErrorKind::NA));
        assert_eq_value_at("=MATCH(35, A1:A5, 0)", &ascending, Value::Error(ErrorKind::NA));

        let descending = [
            ("A1", Value::Number(50.0)),
            ("A2", Value::Number(40.0)),
            ("A3", Value::Number(30.0)),
            ("A4", Value::Number(20.0)),
            ("A5", Value::Number(10.0)),
        ];
        assert_eq_value_at("=MATCH(35, A1:A5, -1)", &descending, Value::Number(2.0));
        assert_eq_value_at("=MATCH(55, A1:A5, -1)", &descending, Value::Error(ErrorKind::NA));
    }

    #[test]
    fn match_supports_wildcards_and_rejects_a_bad_match_type() {
        let cells = [
            ("A1", Value::Text("apple".into())),
            ("A2", Value::Text("banana".into())),
            ("A3", Value::Text("cherry".into())),
        ];
        assert_eq_value_at("=MATCH(\"b*\", A1:A3, 0)", &cells, Value::Number(2.0));
        assert_eq_value_at("=MATCH(\"?herry\", A1:A3, 0)", &cells, Value::Number(3.0));
        assert_eq_value_at("=MATCH(\"fig\", A1:A3, 0)", &cells, Value::Error(ErrorKind::NA));
        assert_eq_value_at("=MATCH(1, A1:A3, 2)", &cells, Value::Error(ErrorKind::Value));
    }

    #[test]
    fn vlookup_searches_the_first_column() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("B1", Value::Text("one".into())),
            ("A2", Value::Number(2.0)),
            ("B2", Value::Text("two".into())),
            ("A3", Value::Number(3.0)),
            ("B3", Value::Text("three".into())),
            ("A4", Value::Number(4.0)),
            ("B4", Value::Text("four".into())),
        ];
        assert_eq_value_at("=VLOOKUP(3, A1:B4, 2, FALSE)", &cells, Value::Text("three".into()));
        assert_eq_value_at("=VLOOKUP(2.5, A1:B4, 2)", &cells, Value::Text("two".into()));
        assert_eq_value_at("=VLOOKUP(4, A1:B4, 2, TRUE)", &cells, Value::Text("four".into()));
        assert_eq_value_at("=VLOOKUP(0, A1:B4, 2)", &cells, Value::Error(ErrorKind::NA));
        assert_eq_value_at("=VLOOKUP(9, A1:B4, 2, FALSE)", &cells, Value::Error(ErrorKind::NA));
        assert_eq_value_at("=VLOOKUP(3, A1:B4, 3)", &cells, Value::Error(ErrorKind::Ref));
        assert_eq_value_at("=VLOOKUP(3, A1:B4, 0)", &cells, Value::Error(ErrorKind::Ref));
    }

    #[test]
    fn vlookup_exact_supports_wildcards_on_text_keys() {
        let cells = [
            ("A1", Value::Text("apple".into())),
            ("B1", Value::Number(1.0)),
            ("A2", Value::Text("banana".into())),
            ("B2", Value::Number(2.0)),
            ("A3", Value::Text("cherry".into())),
            ("B3", Value::Number(3.0)),
        ];
        assert_eq_value_at("=VLOOKUP(\"b*\", A1:B3, 2, FALSE)", &cells, Value::Number(2.0));
        assert_eq_value_at("=VLOOKUP(\"BANANA\", A1:B3, 2, FALSE)", &cells, Value::Number(2.0));
        assert_eq_value_at("=VLOOKUP(\"z*\", A1:B3, 2, FALSE)", &cells, Value::Error(ErrorKind::NA));
    }

    #[test]
    fn hlookup_searches_the_first_row() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("B1", Value::Number(2.0)),
            ("C1", Value::Number(3.0)),
            ("A2", Value::Text("one".into())),
            ("B2", Value::Text("two".into())),
            ("C2", Value::Text("three".into())),
        ];
        assert_eq_value_at("=HLOOKUP(2, A1:C2, 2, FALSE)", &cells, Value::Text("two".into()));
        assert_eq_value_at("=HLOOKUP(2.5, A1:C2, 2)", &cells, Value::Text("two".into()));
        assert_eq_value_at("=HLOOKUP(9, A1:C2, 2)", &cells, Value::Text("three".into()));
        assert_eq_value_at("=HLOOKUP(0, A1:C2, 2)", &cells, Value::Error(ErrorKind::NA));
        assert_eq_value_at("=HLOOKUP(2, A1:C2, 3)", &cells, Value::Error(ErrorKind::Ref));
    }

    #[test]
    fn xlookup_matches_position_by_position() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("B1", Value::Text("one".into())),
            ("B2", Value::Text("two".into())),
            ("B3", Value::Text("three".into())),
        ];
        assert_eq_value_at("=XLOOKUP(2, A1:A3, B1:B3)", &cells, Value::Text("two".into()));
        assert_eq_value_at("=XLOOKUP(9, A1:A3, B1:B3, \"nope\")", &cells, Value::Text("nope".into()));
        assert_eq_value_at("=XLOOKUP(9, A1:A3, B1:B3)", &cells, Value::Error(ErrorKind::NA));
        assert_eq_value_at("=XLOOKUP(\"TWO\", A1:A3, B1:B3)", &cells, Value::Error(ErrorKind::NA));
    }

    #[test]
    fn xlookup_rejects_mismatched_or_scalar_ranges() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("B1", Value::Number(10.0)),
            ("B2", Value::Number(20.0)),
        ];
        assert_eq_value_at("=XLOOKUP(2, A1:A3, B1:B2)", &cells, Value::Error(ErrorKind::Value));
        assert_eq_value_at("=XLOOKUP(2, A1:A3, 5)", &cells, Value::Error(ErrorKind::Value));
        assert_eq_value_at("=XLOOKUP(2, 5, B1:B2)", &cells, Value::Error(ErrorKind::Value));
    }
}
