//! Built-in functions.
//!
//! # Range versus scalar arguments
//!
//! Spreadsheet functions treat arguments that *are* ranges differently from
//! arguments typed directly: `SUM(A1:A3)` ignores text and logical values inside
//! the range, while `SUM("text")` is a `#VALUE!`. Likewise `MIN(TRUE, 2)` is `1`
//! because `TRUE` coerces, but a `TRUE` sitting in a range is ignored. That rule is
//! implemented in [`collect_numbers`] and applies to `SUM`, `AVERAGE`, `COUNT`,
//! `MIN` and `MAX`.
//!
//! `IF` never reaches this module: it is compiled to branches so that only the
//! taken side is evaluated (see [`crate::compile`]).

use crate::addr::RangeRef;
use crate::compile::FuncId;
use crate::error::ErrorKind;
use crate::eval::{Operand, ValueSource};
use crate::value::Value;

/// Call a built-in function with already-evaluated arguments.
pub fn dispatch(func: FuncId, args: &[Operand], source: &dyn ValueSource) -> Value {
    match func {
        FuncId::Sum => numeric_aggregate(args, source, Aggregate::Sum),
        FuncId::Average => numeric_aggregate(args, source, Aggregate::Average),
        FuncId::Min => numeric_aggregate(args, source, Aggregate::Min),
        FuncId::Max => numeric_aggregate(args, source, Aggregate::Max),
        FuncId::Count => count(args, source),
        FuncId::Concat => concat(args, source),
        // IF is compiled to jumps, so this arm is unreachable in practice; it is
        // only here so that the match is total.
        FuncId::If => Value::Error(ErrorKind::Value),
    }
}

#[derive(Clone, Copy)]
enum Aggregate {
    Sum,
    Average,
    Min,
    Max,
}

fn numeric_aggregate(args: &[Operand], source: &dyn ValueSource, aggregate: Aggregate) -> Value {
    let mut numbers = Vec::new();
    if let Err(kind) = collect_numbers(args, source, &mut numbers) {
        return Value::Error(kind);
    }

    match aggregate {
        Aggregate::Sum => Value::finite_number(numbers.iter().sum()),
        Aggregate::Average => {
            if numbers.is_empty() {
                // Like Excel: averaging nothing is a division by zero.
                Value::Error(ErrorKind::Div0)
            } else {
                Value::finite_number(numbers.iter().sum::<f64>() / numbers.len() as f64)
            }
        }
        // Excel returns 0 for MIN/MAX of an empty set.
        Aggregate::Min if numbers.is_empty() => Value::Number(0.0),
        Aggregate::Max if numbers.is_empty() => Value::Number(0.0),
        Aggregate::Min => Value::finite_number(numbers.iter().copied().fold(f64::INFINITY, f64::min)),
        Aggregate::Max => Value::finite_number(numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
    }
}

fn count(args: &[Operand], source: &dyn ValueSource) -> Value {
    let mut numbers = Vec::new();
    match collect_numbers(args, source, &mut numbers) {
        Ok(()) => Value::Number(numbers.len() as f64),
        Err(kind) => Value::Error(kind),
    }
}

fn concat(args: &[Operand], source: &dyn ValueSource) -> Value {
    let mut out = String::new();
    for arg in args {
        match arg {
            Operand::Value(value) => {
                if let Value::Error(kind) = value {
                    return Value::Error(*kind);
                }
                // Empty scalars contribute nothing.
                out.push_str(&value.as_text());
            }
            Operand::Range(range) => {
                if range.bounds().is_none() {
                    return Value::Error(ErrorKind::Ref);
                }
                let mut error = None;
                source.visit_range(*range, &mut |value| match value {
                    Value::Error(kind) if error.is_none() => error = Some(kind),
                    Value::Error(_) => {}
                    other => out.push_str(&other.as_text()),
                });
                if let Some(kind) = error {
                    return Value::Error(kind);
                }
            }
        }
    }
    Value::Text(out)
}

/// Collect the numbers contributed by a list of arguments.
///
/// Errors short-circuit: the first error found anywhere is returned, matching the
/// "one bad cell poisons the aggregate" behaviour spreadsheets have.
fn collect_numbers(
    args: &[Operand],
    source: &dyn ValueSource,
    out: &mut Vec<f64>,
) -> Result<(), ErrorKind> {
    for arg in args {
        match arg {
            Operand::Value(value) => match value {
                Value::Number(n) => out.push(*n),
                // Empty and boolean scalars coerce.
                Value::Empty => out.push(0.0),
                Value::Bool(b) => out.push(if *b { 1.0 } else { 0.0 }),
                Value::Text(_) => return Err(ErrorKind::Value),
                Value::Error(kind) => return Err(*kind),
            },
            Operand::Range(range) => {
                if range.bounds().is_none() {
                    return Err(ErrorKind::Ref);
                }
                let mut error = None;
                let mut values = Vec::new();
                source.visit_range(*range, &mut |value| match value {
                    Value::Number(n) => values.push(n),
                    Value::Error(kind) if error.is_none() => error = Some(kind),
                    // Text, booleans and blanks inside a range are ignored.
                    _ => {}
                });
                if let Some(kind) = error {
                    return Err(kind);
                }
                out.extend(values);
            }
        }
    }
    Ok(())
}

/// Does `func` consume its arguments as ranges? (Used by documentation and tests.)
pub fn is_aggregate(func: FuncId) -> bool {
    matches!(func, FuncId::Sum | FuncId::Average | FuncId::Count | FuncId::Min | FuncId::Max)
}

/// The range a formula should iterate for a single-range argument.
pub fn range_of(arg: &Operand) -> Option<RangeRef> {
    match arg {
        Operand::Range(r) => Some(*r),
        Operand::Value(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::CellRef;
    use crate::compile::compile_source;
    use crate::eval::evaluate;
    use std::collections::HashMap;

    fn sheet(cells: &[(&str, Value)]) -> HashMap<CellRef, Value> {
        let mut map = HashMap::new();
        for (a1, value) in cells {
            map.insert(CellRef::parse_a1(a1).unwrap(), value.clone());
        }
        map
    }

    fn eval_at(src: &str, cells: &[(&str, Value)], host: &str) -> Value {
        let program = compile_source(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        let source = sheet(cells);
        evaluate(&program, &source, CellRef::parse_a1(host).unwrap())
    }

    fn eval(src: &str) -> Value {
        eval_at(src, &[], "A1")
    }

    #[test]
    fn sum_adds_numbers_and_ignores_text_in_ranges() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Text("n/a".into())),
            ("A3", Value::Number(2.0)),
            ("A4", Value::Bool(true)),
        ];
        assert_eq!(eval_at("=SUM(A1:A4)", &cells, "B1"), Value::Number(3.0));
        assert_eq!(eval_at("=SUM(A1:A4, 4)", &cells, "B1"), Value::Number(7.0));
        // Scalars coerce: TRUE is 1.
        assert_eq!(eval("=SUM(TRUE, 2)"), Value::Number(3.0));
        // Text as a direct argument is a type error.
        assert_eq!(eval("=SUM(\"nope\")"), Value::Error(ErrorKind::Value));
        assert_eq!(eval("=SUM()"), Value::Number(0.0));
    }

    #[test]
    fn sum_propagates_errors_found_in_ranges() {
        let cells = [("A1", Value::Number(1.0)), ("A2", Value::Error(ErrorKind::Div0))];
        assert_eq!(eval_at("=SUM(A1:A2)", &cells, "B1"), Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn average_divides_by_the_count_of_numbers() {
        let cells = [("A1", Value::Number(2.0)), ("A2", Value::Number(4.0))];
        assert_eq!(eval_at("=AVERAGE(A1:A2)", &cells, "B1"), Value::Number(3.0));
        assert_eq!(eval("=AVERAGE()"), Value::Error(ErrorKind::Div0));
        assert_eq!(eval("=AVERAGE(1, \"x\")"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn count_counts_numbers_only() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Text("x".into())),
            ("A3", Value::Bool(true)),
            ("A4", Value::Empty),
            ("A5", Value::Number(2.5)),
        ];
        assert_eq!(eval_at("=COUNT(A1:A5)", &cells, "B1"), Value::Number(2.0));
        assert_eq!(eval("=COUNT()"), Value::Number(0.0));
    }

    #[test]
    fn min_and_max_ignore_non_numbers() {
        let cells = [
            ("A1", Value::Number(5.0)),
            ("A2", Value::Number(-1.0)),
            ("A3", Value::Text("x".into())),
            ("A4", Value::Bool(true)),
        ];
        assert_eq!(eval_at("=MIN(A1:A4)", &cells, "B1"), Value::Number(-1.0));
        assert_eq!(eval_at("=MAX(A1:A4)", &cells, "B1"), Value::Number(5.0));
        // Excel returns 0 when there is nothing to compare.
        assert_eq!(eval("=MIN()"), Value::Number(0.0));
        assert_eq!(eval("=MAX(\"x\")"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn concat_joins_text_and_formats_numbers() {
        assert_eq!(eval("=CONCAT(\"a\", \"b\")"), Value::Text("ab".into()));
        assert_eq!(eval("=CONCAT(1, \"-\", 2.5)"), Value::Text("1-2.5".into()));
        assert_eq!(eval("=CONCAT(TRUE)"), Value::Text("TRUE".into()));
        assert_eq!(eval("=CONCAT()"), Value::Text(String::new()));
        let cells = [("A1", Value::Text("x".into())), ("A2", Value::Empty), ("A3", Value::Text("y".into()))];
        assert_eq!(eval_at("=CONCAT(A1:A3)", &cells, "B1"), Value::Text("xy".into()));
    }

    #[test]
    fn concat_propagates_errors() {
        assert_eq!(eval("=CONCAT(\"a\", #N/A)"), Value::Error(ErrorKind::NA));
        let cells = [("A1", Value::Error(ErrorKind::Num))];
        assert_eq!(eval_at("=CONCAT(A1:A1)", &cells, "B1"), Value::Error(ErrorKind::Num));
    }

    #[test]
    fn aggregates_over_an_entire_column_use_sparse_iteration() {
        // A whole-column range is a million cells if expanded. Only stored cells
        // are visited, so this stays instant.
        let cells = [("A1", Value::Number(1.0)), ("A500000", Value::Number(2.0))];
        assert_eq!(eval_at("=SUM(A1:A1048576)", &cells, "B1"), Value::Number(3.0));
        assert_eq!(eval_at("=COUNT(A1:A1048576)", &cells, "B1"), Value::Number(2.0));
    }

    #[test]
    fn functions_are_case_insensitive() {
        assert_eq!(eval("=sum(1,2)"), Value::Number(3.0));
        assert_eq!(eval("=Sum(1,2)"), Value::Number(3.0));
    }

    #[test]
    fn unknown_functions_are_name_errors() {
        assert_eq!(eval("=NOPE(1)"), Value::Error(ErrorKind::Name));
        assert_eq!(eval("=TOTALLY_NOT_A_FUNCTION"), Value::Error(ErrorKind::Name));
    }

    #[test]
    fn aggregate_helpers_agree_with_dispatch() {
        assert!(is_aggregate(FuncId::Sum));
        assert!(!is_aggregate(FuncId::Concat));
        assert_eq!(range_of(&Operand::Value(Value::Empty)), None);
    }
}
