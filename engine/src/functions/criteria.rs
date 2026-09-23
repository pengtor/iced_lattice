use std::cmp::Ordering;

use super::*;

pub(super) fn dispatch(
    func: FuncId,
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
) -> Option<Value> {
    Some(match func {
        FuncId::SumIf => sumif(args, source, host),
        FuncId::CountIf => countif(args, source, host),
        FuncId::AverageIf => averageif(args, source, host),
        FuncId::SumIfs => ifs(args, source, host, Ifs::Sum),
        FuncId::CountIfs => ifs(args, source, host, Ifs::Count),
        FuncId::AverageIfs => ifs(args, source, host, Ifs::Average),
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

enum Test {
    Number { op: Cmp, value: f64 },
    NumberOrText { number: f64, pattern: String },
    Text { op: Cmp, value: String },
    Bool(bool),
}

impl Test {
    fn matches(&self, cell: &Value) -> Result<bool, ErrorKind> {
        if let Value::Error(kind) = cell {
            return Err(*kind);
        }
        Ok(match self {
            Test::Number { op, value } => match cell {
                Value::Number(n) => compare_number(*n, *value, *op),
                _ => false,
            },
            Test::NumberOrText { number, pattern } => match cell {
                Value::Number(n) => *n == *number,
                Value::Text(text) => wildcard_match(pattern, text),
                _ => false,
            },
            Test::Text { op, value } => match cell {
                Value::Text(text) => compare_text(text, value, *op),
                _ => false,
            },
            Test::Bool(expected) => match cell {
                Value::Bool(actual) => actual == expected,
                _ => false,
            },
        })
    }
}

fn compile_criterion(value: &Value) -> Result<Test, ErrorKind> {
    match value {
        Value::Error(kind) => Err(*kind),
        Value::Number(n) => Ok(Test::Number { op: Cmp::Eq, value: *n }),
        Value::Bool(b) => Ok(Test::Bool(*b)),
        Value::Empty => Ok(Test::Number { op: Cmp::Eq, value: 0.0 }),
        Value::Text(raw) => compile_text(raw),
    }
}

fn compile_text(raw: &str) -> Result<Test, ErrorKind> {
    let (explicit_op, operand) = split_operator(raw);
    if explicit_op.is_some() && operand.is_empty() {
        return Err(ErrorKind::Value);
    }
    let op = explicit_op.unwrap_or(Cmp::Eq);

    match parse_number(operand) {
        Some(number) if op == Cmp::Eq => {
            // equality: numeric operand also matches the text spelling
            Ok(Test::NumberOrText { number, pattern: operand.to_string() })
        }
        Some(number) => Ok(Test::Number { op, value: number }),
        None => Ok(Test::Text { op, value: operand.to_string() }),
    }
}

// two-char operators first so ">=1" isn't read as ">"
fn split_operator(raw: &str) -> (Option<Cmp>, &str) {
    const OPERATORS: [(&str, Cmp); 6] = [
        (">=", Cmp::Ge),
        ("<=", Cmp::Le),
        ("<>", Cmp::Ne),
        (">", Cmp::Gt),
        ("<", Cmp::Lt),
        ("=", Cmp::Eq),
    ];
    for (prefix, op) in OPERATORS {
        if let Some(rest) = raw.strip_prefix(prefix) {
            return (Some(op), rest);
        }
    }
    (None, raw)
}

// `f64::parse` accepts "inf"/"NaN"; require finite
fn parse_number(operand: &str) -> Option<f64> {
    let number: f64 = operand.trim().parse().ok()?;
    if number.is_finite() {
        Some(number)
    } else {
        None
    }
}

fn compare_number(cell: f64, operand: f64, op: Cmp) -> bool {
    match op {
        Cmp::Eq => cell == operand,
        Cmp::Ne => cell != operand,
        Cmp::Gt => cell > operand,
        Cmp::Ge => cell >= operand,
        Cmp::Lt => cell < operand,
        Cmp::Le => cell <= operand,
    }
}

fn compare_text(cell: &str, operand: &str, op: Cmp) -> bool {
    if op == Cmp::Eq {
        return wildcard_match(operand, cell);
    }
    let ordering = cell.to_lowercase().cmp(&operand.to_lowercase());
    match op {
        Cmp::Ne => ordering != Ordering::Equal,
        Cmp::Gt => ordering == Ordering::Greater,
        Cmp::Ge => ordering != Ordering::Less,
        Cmp::Lt => ordering == Ordering::Less,
        Cmp::Le => ordering != Ordering::Greater,
        Cmp::Eq => unreachable!("equality is handled above"),
    }
}

#[derive(Clone, Copy)]
enum Ifs {
    Sum,
    Count,
    Average,
}

fn range_bounds(args: &[Operand], i: usize) -> Result<Bounds, ErrorKind> {
    bounds_arg(args, i).ok_or(ErrorKind::Value)
}

fn aligned_range(args: &[Operand], i: usize, expected: Bounds) -> Result<Bounds, ErrorKind> {
    let bounds = range_bounds(args, i)?;
    if bounds.rows() != expected.rows() || bounds.cols() != expected.cols() {
        return Err(ErrorKind::Value);
    }
    Ok(bounds)
}

fn optional_aligned_range(args: &[Operand], i: usize, expected: Bounds) -> Result<Bounds, ErrorKind> {
    match args.get(i) {
        None => Ok(expected),
        Some(_) => aligned_range(args, i, expected),
    }
}

fn criterion_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
) -> Result<Test, ErrorKind> {
    compile_criterion(&value_arg(args, i, source, host))
}

fn offset_of(cell: CellRef, bounds: Bounds) -> u64 {
    u64::from(cell.row - bounds.min_row) * u64::from(bounds.cols())
        + u64::from(cell.col - bounds.min_col)
}

// stored cells only: blanks are never candidates
fn scan(
    source: &dyn ValueSource,
    primary: Bounds,
    pairs: &[(Bounds, Test)],
    value_range: Option<Bounds>,
) -> Result<(f64, u64), ErrorKind> {
    let mut sum = 0.0;
    let mut count = 0u64;
    let mut failure: Option<ErrorKind> = None;

    each_in_bounds(source, primary, &mut |cell, _value| {
        if failure.is_some() {
            return;
        }
        let offset = offset_of(cell, primary);

        // all pairs evaluated, not short-circuited, so errors propagate
        let mut matched = true;
        for (bounds, test) in pairs {
            let candidate = value_at_offset(source, *bounds, offset);
            match test.matches(&candidate) {
                Ok(true) => {}
                Ok(false) => matched = false,
                Err(kind) => {
                    failure = Some(kind);
                    return;
                }
            }
        }
        if !matched {
            return;
        }

        match value_range {
            None => count += 1,
            Some(bounds) => match value_at_offset(source, bounds, offset) {
                Value::Number(n) => {
                    sum += n;
                    count += 1;
                }
                Value::Error(kind) => failure = Some(kind),
                _ => {}
            },
        }
    });

    match failure {
        Some(kind) => Err(kind),
        None => Ok((sum, count)),
    }
}

fn sumif(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let criteria_bounds = match range_bounds(args, 0) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    let test = match criterion_arg(args, 1, source, host) {
        Ok(test) => test,
        Err(kind) => return Value::Error(kind),
    };
    let value_bounds = match optional_aligned_range(args, 2, criteria_bounds) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };

    let pairs = [(criteria_bounds, test)];
    match scan(source, criteria_bounds, &pairs, Some(value_bounds)) {
        Ok((sum, _)) => Value::finite_number(sum),
        Err(kind) => Value::Error(kind),
    }
}

fn countif(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let criteria_bounds = match range_bounds(args, 0) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    let test = match criterion_arg(args, 1, source, host) {
        Ok(test) => test,
        Err(kind) => return Value::Error(kind),
    };

    let pairs = [(criteria_bounds, test)];
    match scan(source, criteria_bounds, &pairs, None) {
        Ok((_, count)) => Value::Number(count as f64),
        Err(kind) => Value::Error(kind),
    }
}

fn averageif(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let criteria_bounds = match range_bounds(args, 0) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    let test = match criterion_arg(args, 1, source, host) {
        Ok(test) => test,
        Err(kind) => return Value::Error(kind),
    };
    let value_bounds = match optional_aligned_range(args, 2, criteria_bounds) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };

    let pairs = [(criteria_bounds, test)];
    match scan(source, criteria_bounds, &pairs, Some(value_bounds)) {
        Ok((_, 0)) => Value::Error(ErrorKind::Div0),
        Ok((sum, count)) => Value::finite_number(sum / count as f64),
        Err(kind) => Value::Error(kind),
    }
}

fn ifs(args: &[Operand], source: &dyn ValueSource, host: CellRef, kind: Ifs) -> Value {
    let (value_bounds, pair_start) = match kind {
        Ifs::Count => (None, 0),
        Ifs::Sum | Ifs::Average => {
            let bounds = match range_bounds(args, 0) {
                Ok(bounds) => bounds,
                Err(kind) => return Value::Error(kind),
            };
            (Some(bounds), 1)
        }
    };

    let primary = match range_bounds(args, pair_start) {
        Ok(bounds) => bounds,
        Err(kind) => return Value::Error(kind),
    };
    if let Some(bounds) = value_bounds {
        if bounds.rows() != primary.rows() || bounds.cols() != primary.cols() {
            return Value::Error(ErrorKind::Value);
        }
    }

    let mut pairs = Vec::new();
    let mut i = pair_start;
    while i < args.len() {
        let bounds = match aligned_range(args, i, primary) {
            Ok(bounds) => bounds,
            Err(kind) => return Value::Error(kind),
        };
        let test = match criterion_arg(args, i + 1, source, host) {
            Ok(test) => test,
            Err(kind) => return Value::Error(kind),
        };
        pairs.push((bounds, test));
        i += 2;
    }

    match scan(source, primary, &pairs, value_bounds) {
        Ok((sum, count)) => match kind {
            Ifs::Count => Value::Number(count as f64),
            Ifs::Sum => Value::finite_number(sum),
            Ifs::Average if count == 0 => Value::Error(ErrorKind::Div0),
            Ifs::Average => Value::finite_number(sum / count as f64),
        },
        Err(kind) => Value::Error(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{eval, eval_at};
    use super::*;

    #[track_caller]
    fn assert_eq_value_at(src: &str, cells: &[(&str, Value)], expected: Value) {
        assert_eq!(eval_at(src, cells, "E1"), expected, "{src}");
    }

    #[test]
    fn countif_matches_exactly_and_with_wildcards() {
        let cells = [
            ("A1", Value::Text("apple".into())),
            ("A2", Value::Text("avocado".into())),
            ("A3", Value::Text("banana".into())),
            ("A4", Value::Text("cherry".into())),
        ];
        assert_eq_value_at("=COUNTIF(A1:A4, \"a*\")", &cells, Value::Number(2.0));
        assert_eq_value_at("=COUNTIF(A1:A4, \"banana\")", &cells, Value::Number(1.0));
        assert_eq_value_at("=COUNTIF(A1:A4, \"?anana\")", &cells, Value::Number(1.0));
        assert_eq_value_at("=COUNTIF(A1:A4, \"BANANA\")", &cells, Value::Number(1.0));
        assert_eq_value_at("=COUNTIF(A1:A4, \"z*\")", &cells, Value::Number(0.0));
    }

    #[test]
    fn countif_numeric_criteria_never_match_text_cells() {
        let cells = [
            ("A1", Value::Number(5.0)),
            ("A2", Value::Text("20".into())),
            ("A3", Value::Number(20.0)),
        ];
        assert_eq_value_at("=COUNTIF(A1:A3, \">10\")", &cells, Value::Number(1.0));

        let numbers = [
            ("A1", Value::Number(5.0)),
            ("A2", Value::Number(10.0)),
            ("A3", Value::Number(20.0)),
            ("A4", Value::Number(30.0)),
        ];
        assert_eq_value_at("=COUNTIF(A1:A4, 10)", &numbers, Value::Number(1.0));
        assert_eq_value_at("=COUNTIF(A1:A4, \">=10\")", &numbers, Value::Number(3.0));
        assert_eq_value_at("=COUNTIF(A1:A4, \"<>10\")", &numbers, Value::Number(3.0));
    }

    #[test]
    fn countif_equality_accepts_text_that_spells_a_number() {
        let cells = [("A1", Value::Number(10.0)), ("A2", Value::Text("10".into()))];
        assert_eq_value_at("=COUNTIF(A1:A2, \"10\")", &cells, Value::Number(2.0));
        assert_eq_value_at("=COUNTIF(A1:A2, 10)", &cells, Value::Number(1.0));
    }

    #[test]
    fn countif_booleans_are_named_by_booleans_not_text() {
        let cells = [
            ("A1", Value::Bool(true)),
            ("A2", Value::Bool(false)),
            ("A3", Value::Bool(true)),
        ];
        assert_eq_value_at("=COUNTIF(A1:A3, TRUE)", &cells, Value::Number(2.0));
        assert_eq_value_at("=COUNTIF(A1:A3, FALSE)", &cells, Value::Number(1.0));
        assert_eq_value_at("=COUNTIF(A1:A3, \"TRUE\")", &cells, Value::Number(0.0));
    }

    #[test]
    fn countif_never_matches_a_blank_and_rejects_an_empty_operator() {
        let cells = [
            ("A1", Value::Text("x".into())),
            ("A2", Value::Text("y".into())),
            ("A3", Value::Text("z".into())),
        ];
        assert_eq_value_at("=COUNTIF(A1:A9, \"\")", &cells, Value::Number(0.0));
        assert_eq_value_at("=COUNTIF(A1:A3, \">\")", &cells, Value::Error(ErrorKind::Value));
        assert_eq!(eval("=COUNTIF(5, \">1\")"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn sumif_sums_matching_cells_with_and_without_a_sum_range() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("A4", Value::Number(4.0)),
            ("B1", Value::Number(10.0)),
            ("B2", Value::Number(20.0)),
            ("B3", Value::Number(30.0)),
            ("B4", Value::Number(40.0)),
        ];
        assert_eq_value_at("=SUMIF(A1:A4, \">2\")", &cells, Value::Number(7.0));
        assert_eq_value_at("=SUMIF(A1:A4, \">2\", B1:B4)", &cells, Value::Number(70.0));
        assert_eq_value_at("=SUMIF(A1:A4, 3, B1:B4)", &cells, Value::Number(30.0));
        assert_eq_value_at("=SUMIF(A1:A4, \">100\", B1:B4)", &cells, Value::Number(0.0));
    }

    #[test]
    fn sumif_ignores_text_but_propagates_an_error_in_a_summed_row() {
        let text_in_sum = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("B1", Value::Text("x".into())),
            ("B2", Value::Number(20.0)),
        ];
        assert_eq_value_at("=SUMIF(A1:A2, \">0\", B1:B2)", &text_in_sum, Value::Number(20.0));

        let bad = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("B1", Value::Number(10.0)),
            ("B2", Value::Error(ErrorKind::NA)),
        ];
        assert_eq_value_at("=SUMIF(A1:A2, \">0\", B1:B2)", &bad, Value::Error(ErrorKind::NA));
    }

    #[test]
    fn sumif_rejects_a_mismatched_sum_range() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("A4", Value::Number(4.0)),
            ("B1", Value::Number(10.0)),
            ("B2", Value::Number(20.0)),
            ("B3", Value::Number(30.0)),
        ];
        assert_eq_value_at("=SUMIF(A1:A4, \">0\", B1:B3)", &cells, Value::Error(ErrorKind::Value));
    }

    #[test]
    fn averageif_divides_and_reports_no_matches() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("A4", Value::Number(4.0)),
            ("B1", Value::Number(10.0)),
            ("B2", Value::Number(20.0)),
            ("B3", Value::Number(30.0)),
            ("B4", Value::Number(40.0)),
        ];
        assert_eq_value_at("=AVERAGEIF(A1:A4, \">2\", B1:B4)", &cells, Value::Number(35.0));
        assert_eq_value_at("=AVERAGEIF(A1:A4, \">100\", B1:B4)", &cells, Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn countifs_requires_the_criteria_to_line_up() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("A4", Value::Number(4.0)),
            ("B1", Value::Text("a".into())),
            ("B2", Value::Text("b".into())),
            ("B3", Value::Text("a".into())),
            ("B4", Value::Text("b".into())),
        ];
        assert_eq_value_at("=COUNTIFS(A1:A4, \">1\", B1:B4, \"a\")", &cells, Value::Number(1.0));
        assert_eq_value_at("=COUNTIFS(A1:A4, \">0\", B1:B4, \"b\")", &cells, Value::Number(2.0));
        assert_eq_value_at("=COUNTIFS(A1:A4, \">100\", B1:B4, \"b\")", &cells, Value::Number(0.0));
    }

    #[test]
    fn sumifs_and_averageifs_aggregate_the_matching_positions() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("A4", Value::Number(4.0)),
            ("B1", Value::Text("a".into())),
            ("B2", Value::Text("b".into())),
            ("B3", Value::Text("a".into())),
            ("B4", Value::Text("b".into())),
            ("C1", Value::Number(10.0)),
            ("C2", Value::Number(20.0)),
            ("C3", Value::Number(30.0)),
            ("C4", Value::Number(40.0)),
        ];
        assert_eq_value_at("=SUMIFS(C1:C4, A1:A4, \">1\", B1:B4, \"a\")", &cells, Value::Number(30.0));
        assert_eq_value_at("=SUMIFS(C1:C4, A1:A4, \">0\", B1:B4, \"b\")", &cells, Value::Number(60.0));
        assert_eq_value_at("=AVERAGEIFS(C1:C4, A1:A4, \">1\")", &cells, Value::Number(30.0));
        assert_eq_value_at("=AVERAGEIFS(C1:C4, A1:A4, \">100\")", &cells, Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn the_ifs_forms_reject_mismatched_dimensions() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Number(2.0)),
            ("A3", Value::Number(3.0)),
            ("A4", Value::Number(4.0)),
            ("B1", Value::Text("a".into())),
            ("B2", Value::Text("b".into())),
            ("C1", Value::Number(10.0)),
            ("C2", Value::Number(20.0)),
            ("C3", Value::Number(30.0)),
            ("C4", Value::Number(40.0)),
        ];
        assert_eq_value_at("=COUNTIFS(A1:A4, \">0\", B1:B2, \"a\")", &cells, Value::Error(ErrorKind::Value));
        assert_eq_value_at("=SUMIFS(C1:C4, A1:A4, \">0\", B1:B2, \"a\")", &cells, Value::Error(ErrorKind::Value));
        assert_eq_value_at("=AVERAGEIFS(C1:C3, A1:A4, \">1\")", &cells, Value::Error(ErrorKind::Value));
    }
}
