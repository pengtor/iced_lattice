
use crate::addr::{Bounds, CellRef, RangeRef};
use crate::compile::FuncId;
use crate::error::ErrorKind;
use crate::eval::{Operand, ValueSource, DENSE_RANGE_LIMIT};
use crate::value::Value;

pub mod criteria;
pub mod datetime;
pub mod logic;
pub mod lookup;
pub mod math;
pub mod stats;
pub mod text;

type Router = fn(FuncId, &[Operand], &dyn ValueSource, CellRef) -> Option<Value>;

const ROUTERS: &[Router] = &[
    aggregates,
    logic::dispatch,
    math::dispatch,
    text::dispatch,
    lookup::dispatch,
    criteria::dispatch,
    stats::dispatch,
    datetime::dispatch,
];

pub fn dispatch(func: FuncId, args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    for route in ROUTERS {
        if let Some(value) = route(func, args, source, host) {
            return value;
        }
    }
    match func {
        FuncId::If | FuncId::Ifs | FuncId::IfError | FuncId::IfNa => Value::Error(ErrorKind::Value),
        _ => Value::Error(ErrorKind::Name),
    }
}

fn aggregates(func: FuncId, args: &[Operand], source: &dyn ValueSource, _host: CellRef) -> Option<Value> {
    Some(match func {
        FuncId::Sum => numeric_aggregate(args, source, Aggregate::Sum),
        FuncId::Average => numeric_aggregate(args, source, Aggregate::Average),
        FuncId::Min => numeric_aggregate(args, source, Aggregate::Min),
        FuncId::Max => numeric_aggregate(args, source, Aggregate::Max),
        FuncId::Count => count(args, source),
        FuncId::Concat => concat(args, source),
        _ => return None,
    })
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
                Value::Error(ErrorKind::Div0)
            } else {
                Value::finite_number(numbers.iter().sum::<f64>() / numbers.len() as f64)
            }
        }
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

// first error found anywhere is returned, poisoning the aggregate
pub(super) fn collect_numbers(
    args: &[Operand],
    source: &dyn ValueSource,
    out: &mut Vec<f64>,
) -> Result<(), ErrorKind> {
    for arg in args {
        match arg {
            Operand::Value(value) => match value {
                Value::Number(n) => out.push(*n),
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

pub fn is_aggregate(func: FuncId) -> bool {
    matches!(
        func,
        FuncId::Sum
            | FuncId::Average
            | FuncId::Count
            | FuncId::Min
            | FuncId::Max
            | FuncId::Median
            | FuncId::Mode
            | FuncId::StDev
            | FuncId::Var
    )
}

pub fn range_of(arg: &Operand) -> Option<RangeRef> {
    match arg {
        Operand::Range(r) => Some(*r),
        Operand::Value(_) => None,
    }
}

// ranges collapse by implicit intersection, like operator contexts
pub(super) fn value_arg(args: &[Operand], i: usize, source: &dyn ValueSource, host: CellRef) -> Value {
    args.get(i)
        .cloned()
        .unwrap_or(Operand::Value(Value::Empty))
        .scalar(source, host)
}

pub(super) fn number_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
) -> Result<f64, ErrorKind> {
    value_arg(args, i, source, host).as_number()
}

pub(super) fn opt_number_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
    default: f64,
) -> Result<f64, ErrorKind> {
    match args.get(i) {
        None => Ok(default),
        Some(_) => number_arg(args, i, source, host),
    }
}

pub(super) fn count_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
) -> Result<i64, ErrorKind> {
    Ok(number_arg(args, i, source, host)?.trunc() as i64)
}

pub(super) fn opt_count_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
    default: i64,
) -> Result<i64, ErrorKind> {
    match args.get(i) {
        None => Ok(default),
        Some(_) => count_arg(args, i, source, host),
    }
}

pub(super) fn text_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
) -> Result<String, ErrorKind> {
    match value_arg(args, i, source, host) {
        Value::Error(kind) => Err(kind),
        other => Ok(other.as_text()),
    }
}

pub(super) fn bool_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
) -> Result<bool, ErrorKind> {
    value_arg(args, i, source, host).as_bool()
}

pub(super) fn opt_bool_arg(
    args: &[Operand],
    i: usize,
    source: &dyn ValueSource,
    host: CellRef,
    default: bool,
) -> Result<bool, ErrorKind> {
    match args.get(i) {
        None => Ok(default),
        Some(_) => bool_arg(args, i, source, host),
    }
}

pub(super) fn range_arg(args: &[Operand], i: usize) -> Option<RangeRef> {
    match args.get(i) {
        Some(Operand::Range(r)) => Some(*r),
        _ => None,
    }
}

pub(super) fn bounds_arg(args: &[Operand], i: usize) -> Option<Bounds> {
    range_arg(args, i)?.bounds()
}

// only stored cells exist; small rects dense, large rects sparse
pub(super) fn each_in_bounds(
    source: &dyn ValueSource,
    bounds: Bounds,
    visit: &mut dyn FnMut(CellRef, Value),
) {
    if bounds.len() <= DENSE_RANGE_LIMIT {
        for cell in bounds.iter_cells() {
            let value = source.value(cell);
            if !value.is_empty() {
                visit(cell, value);
            }
        }
    } else {
        let mut stored = Vec::new();
        source.each_stored(&mut |cell, value| {
            if bounds.contains(cell) {
                stored.push((cell, value.clone()));
            }
        });
        stored.sort_by_key(|(cell, _)| (cell.row, cell.col));
        for (cell, value) in stored {
            visit(cell, value);
        }
    }
}

pub(super) fn cells_in_order(source: &dyn ValueSource, bounds: Bounds) -> Vec<(u64, Value)> {
    let cols = bounds.cols() as u64;
    let mut out = Vec::new();
    each_in_bounds(source, bounds, &mut |cell, value| {
        let offset = u64::from(cell.row - bounds.min_row) * cols + u64::from(cell.col - bounds.min_col);
        out.push((offset, value));
    });
    out
}

pub(super) fn value_at_offset(source: &dyn ValueSource, bounds: Bounds, offset: u64) -> Value {
    if offset >= bounds.len() {
        return Value::Empty;
    }
    let cols = u64::from(bounds.cols());
    let row = bounds.min_row + (offset / cols) as u32;
    let col = bounds.min_col + (offset % cols) as u32;
    source.value(CellRef::new(row, col))
}

enum Pat {
    Any,
    One,
    Literal(char),
}

fn compile_pattern(pattern: &str) -> Vec<Pat> {
    let mut out = Vec::new();
    let lowered = pattern.to_lowercase();
    let mut chars = lowered.chars();
    while let Some(c) = chars.next() {
        match c {
            '~' => out.push(Pat::Literal(chars.next().unwrap_or('~'))),
            '*' => out.push(Pat::Any),
            '?' => out.push(Pat::One),
            other => out.push(Pat::Literal(other)),
        }
    }
    out
}

fn glob_at(pattern: &[Pat], text: &[char], whole: bool) -> bool {
    let (mut p, mut t) = (0usize, 0usize);
    // iterative matcher with single backtrack: star patterns cannot blow up
    let mut star: Option<(usize, usize)> = None;

    fn retry(star: &mut Option<(usize, usize)>, p: &mut usize, t: &mut usize, len: usize) -> bool {
        match *star {
            Some((star_p, star_t)) if star_t < len => {
                *p = star_p + 1;
                *t = star_t + 1;
                *star = Some((star_p, star_t + 1));
                true
            }
            _ => false,
        }
    }

    loop {
        match pattern.get(p) {
            Some(Pat::Any) => {
                star = Some((p, t));
                p += 1;
            }
            Some(Pat::One) if t < text.len() => {
                p += 1;
                t += 1;
            }
            Some(Pat::Literal(c)) if t < text.len() && *c == text[t] => {
                p += 1;
                t += 1;
            }
            None if !whole || t == text.len() => return true,
            _ => {
                if !retry(&mut star, &mut p, &mut t, text.len()) {
                    return false;
                }
            }
        }
    }
}

fn lower_chars(text: &str) -> Vec<char> {
    text.to_lowercase().chars().collect()
}

pub(super) fn wildcard_match(pattern: &str, text: &str) -> bool {
    glob_at(&compile_pattern(pattern), &lower_chars(text), true)
}

pub(super) fn wildcard_find(pattern: &str, text: &str, start: usize) -> Option<usize> {
    let pattern = compile_pattern(pattern);
    let haystack = lower_chars(text);
    (start..=haystack.len()).find(|i| glob_at(&pattern, &haystack[*i..], false))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Rounding {
    HalfAwayFromZero,
    Up,
    Down,
}

// rounds decimal digits, not binary scaling (2.675*100 misbehaves)
pub(super) fn round_decimal(x: f64, places: i32, mode: Rounding) -> f64 {
    if !x.is_finite() || x == 0.0 {
        return x;
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let abs = x.abs();
    if !(1e-300..1e21).contains(&abs) || places >= 17 {
        return x;
    }

    let text = format!("{abs}");
    let (int_part, frac_part) = match text.split_once('.') {
        Some((i, f)) => (i, f),
        None => (text.as_str(), ""),
    };
    let mut digits: Vec<u8> = int_part.bytes().chain(frac_part.bytes()).map(|b| b - b'0').collect();
    let mut point = int_part.len() as i32;
    let leading = digits.iter().take_while(|d| **d == 0).count();
    digits.drain(..leading);
    point -= leading as i32;
    while digits.last() == Some(&0) {
        digits.pop();
    }
    if digits.is_empty() {
        return 0.0;
    }

    let keep = point + places;
    let round_up = if keep >= digits.len() as i32 {
        return x;
    } else if keep < 0 {
        mode == Rounding::Up
    } else if keep == 0 {
        let first = digits[0];
        match mode {
            Rounding::HalfAwayFromZero => first >= 5,
            Rounding::Up => true,
            Rounding::Down => false,
        }
    } else {
        let first = digits[keep as usize];
        match mode {
            Rounding::HalfAwayFromZero => first >= 5,
            Rounding::Up => digits[keep as usize..].iter().any(|d| *d != 0),
            Rounding::Down => false,
        }
    };

    let mut kept: Vec<u8> = if keep <= 0 { Vec::new() } else { digits[..keep as usize].to_vec() };
    if round_up {
        let mut i = kept.len();
        loop {
            if i == 0 {
                kept.insert(0, 1);
                break;
            }
            i -= 1;
            if kept[i] == 9 {
                kept[i] = 0;
            } else {
                kept[i] += 1;
                break;
            }
        }
    }
    if kept.is_empty() {
        return 0.0;
    }

    let kept_digits: String = kept.iter().map(|d| (b'0' + d) as char).collect();
    let magnitude: f64 = format!("{kept_digits}e{}", point - keep).parse().unwrap_or(0.0);
    sign * magnitude
}

pub(super) fn fixed_decimal(x: f64, places: usize) -> String {
    let rounded = round_decimal(x, places as i32, Rounding::HalfAwayFromZero);
    format!("{rounded:.places$}")
}

pub(super) fn group_thousands(text: &str) -> String {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text),
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (rest, None),
    };
    let mut grouped = String::with_capacity(int_part.len() + int_part.len() / 3 + 4);
    for (i, c) in int_part.chars().enumerate() {
        if i > 0 && (int_part.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac_part {
        Some(frac) => format!("{sign}{grouped}.{frac}"),
        None => format!("{sign}{grouped}"),
    }
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use crate::compile::compile_source;
    use crate::eval::evaluate;
    use std::cell::RefCell;
    use std::collections::HashMap;

    pub fn sheet(cells: &[(&str, Value)]) -> HashMap<CellRef, Value> {
        let mut map = HashMap::new();
        for (a1, value) in cells {
            map.insert(CellRef::parse_a1(a1).unwrap(), value.clone());
        }
        map
    }

    pub fn eval_with(src: &str, source: &dyn ValueSource, host: &str) -> Value {
        let program = compile_source(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        evaluate(&program, source, CellRef::parse_a1(host).unwrap())
    }

    pub fn eval_at(src: &str, cells: &[(&str, Value)], host: &str) -> Value {
        eval_with(src, &sheet(cells), host)
    }

    pub fn eval(src: &str) -> Value {
        eval_at(src, &[], "A1")
    }

    #[track_caller]
    pub fn assert_error(src: &str, expected: ErrorKind) {
        match eval(src) {
            Value::Error(kind) => assert_eq!(kind, expected, "{src}"),
            other => panic!("{src}: expected {expected}, got {other:?}"),
        }
    }

    #[track_caller]
    pub fn assert_eq_value(src: &str, expected: Value) {
        assert_eq!(eval(src), expected, "{src}");
    }

    pub struct CountingSource {
        cells: HashMap<CellRef, Value>,
        reads: RefCell<Vec<CellRef>>,
    }

    impl CountingSource {
        pub fn new(cells: &[(&str, Value)]) -> Self {
            CountingSource { cells: sheet(cells), reads: RefCell::new(Vec::new()) }
        }

        pub fn read_count(&self, cell: &str) -> usize {
            let cell = CellRef::parse_a1(cell).unwrap();
            self.reads.borrow().iter().filter(|c| **c == cell).count()
        }
    }

    impl ValueSource for CountingSource {
        fn value(&self, cell: CellRef) -> Value {
            self.reads.borrow_mut().push(cell);
            self.cells.get(&cell).cloned().unwrap_or(Value::Empty)
        }

        fn each_stored(&self, visit: &mut dyn FnMut(CellRef, &Value)) {
            for (cell, value) in &self.cells {
                visit(*cell, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

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
        assert_eq!(eval("=SUM(TRUE, 2)"), Value::Number(3.0));
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

    #[test]
    fn every_name_round_trips_and_no_two_functions_share_one() {
        let mut seen = std::collections::HashSet::new();
        for func in FuncId::ALL {
            let name = func.name();
            assert_eq!(FuncId::from_name(name), Some(*func), "{name}");
            assert_eq!(FuncId::from_name(&name.to_lowercase()), Some(*func), "{name}");
            assert!(seen.insert(name), "{name} is declared twice");
        }
    }

    #[test]
    fn every_function_is_owned_by_exactly_one_category() {
        let source: std::collections::HashMap<CellRef, Value> = std::collections::HashMap::new();
        let host = CellRef::new(0, 0);

        for func in FuncId::ALL {
            let args: Vec<Operand> = (0..func.arity().sample())
                .map(|i| {
                    Operand::Value(if i % 2 == 0 { Value::Number(1.0) } else { Value::Text("x".into()) })
                })
                .collect();
            let owners = ROUTERS.iter().filter(|route| route(*func, &args, &source, host).is_some()).count();
            if matches!(func, FuncId::If | FuncId::Ifs | FuncId::IfError | FuncId::IfNa) {
                assert_eq!(owners, 0, "{func} is compiled to branches and must not be dispatched");
            } else {
                assert_eq!(owners, 1, "{func} is not owned by exactly one category");
            }
        }
    }

    #[test]
    fn arity_rules_reject_what_they_should() {
        assert_eq!(FuncId::If.arity_rule(), "2 or 3 arguments");
        assert_eq!(FuncId::If.accepts(1), false);
        assert_eq!(FuncId::If.accepts(2), true);
        assert_eq!(FuncId::If.accepts(4), false);

        assert_eq!(FuncId::Today.arity_rule(), "no arguments");
        assert_eq!(FuncId::Round.arity_rule(), "1 or 2 arguments");
        assert_eq!(FuncId::Date.arity_rule(), "exactly 3 arguments");
        assert_eq!(FuncId::And.arity_rule(), "at least 1 argument");
        assert_eq!(FuncId::CountIfs.arity_rule(), "an even number of arguments (at least 2)");
        assert_eq!(FuncId::SumIfs.arity_rule(), "an odd number of arguments (at least 3)");

        assert!(FuncId::Ifs.accepts(2));
        assert!(!FuncId::Ifs.accepts(3));
        assert!(FuncId::SumIfs.accepts(3));
        assert!(FuncId::SumIfs.accepts(5));
        assert!(!FuncId::SumIfs.accepts(4));
        assert!(!FuncId::SumIfs.accepts(2));
        assert!(FuncId::CountIfs.accepts(4));
        assert!(!FuncId::CountIfs.accepts(3));
    }

    #[test]
    fn wildcards_match_the_way_a_spreadsheet_says() {
        assert!(wildcard_match("a*", "abc"));
        assert!(wildcard_match("*c", "abc"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("", ""));
        assert!(!wildcard_match("", "a"));
        assert!(!wildcard_match("a?c", "ac"));
        assert!(wildcard_match("ABC", "abc"));
        assert!(wildcard_match("~*", "*"));
        assert!(!wildcard_match("~*", "abc"));
        assert!(wildcard_match("a~?c", "a?c"));
        assert!(wildcard_match(&"*".repeat(64), &"a".repeat(64)));
    }

    #[test]
    fn wildcard_find_is_a_case_insensitive_search() {
        assert_eq!(wildcard_find("b", "abc", 0), Some(1));
        assert_eq!(wildcard_find("B", "abc", 0), Some(1));
        assert_eq!(wildcard_find("z", "abc", 0), None);
        assert_eq!(wildcard_find("a", "abc", 1), None);
        assert_eq!(wildcard_find("b*", "abc", 0), Some(1));
        assert_eq!(wildcard_find("", "abc", 2), Some(2));
    }

    #[test]
    fn decimal_rounding_matches_what_a_user_expects_to_see() {
        use Rounding::*;
        assert_eq!(round_decimal(2.675, 2, HalfAwayFromZero), 2.68);
        assert_eq!(round_decimal(1.005, 2, HalfAwayFromZero), 1.01);
        assert_eq!(round_decimal(0.5, 0, HalfAwayFromZero), 1.0);
        assert_eq!(round_decimal(-0.5, 0, HalfAwayFromZero), -1.0);
        assert_eq!(round_decimal(2.5, 0, HalfAwayFromZero), 3.0);
        assert_eq!(round_decimal(-2.5, 0, HalfAwayFromZero), -3.0);

        assert_eq!(round_decimal(1234.0, -2, HalfAwayFromZero), 1200.0);
        assert_eq!(round_decimal(1250.0, -2, HalfAwayFromZero), 1300.0);
        assert_eq!(round_decimal(3.0, -5, HalfAwayFromZero), 0.0);
        assert_eq!(round_decimal(3.0, -5, Up), 100000.0);

        assert_eq!(round_decimal(2.1, 0, Up), 3.0);
        assert_eq!(round_decimal(-2.1, 0, Up), -3.0);
        assert_eq!(round_decimal(2.0, 0, Up), 2.0);
        assert_eq!(round_decimal(2.9, 0, Down), 2.0);
        assert_eq!(round_decimal(-2.9, 0, Down), -2.0);
        assert_eq!(round_decimal(0.4, 0, Down), 0.0);

        assert_eq!(round_decimal(5.0, 0, HalfAwayFromZero), 5.0);
        assert_eq!(round_decimal(5.0, 9, HalfAwayFromZero), 5.0);
        assert_eq!(round_decimal(0.0, 2, HalfAwayFromZero), 0.0);
        assert_eq!(round_decimal(9.99, 1, HalfAwayFromZero), 10.0);
        assert_eq!(round_decimal(0.999, 2, HalfAwayFromZero), 1.0);
    }

    #[test]
    fn fixed_decimals_and_grouping_render_like_a_cell() {
        assert_eq!(fixed_decimal(1.0, 2), "1.00");
        assert_eq!(fixed_decimal(1.005, 2), "1.01");
        assert_eq!(fixed_decimal(-2.5, 0), "-3");
        assert_eq!(group_thousands("1234.5"), "1,234.5");
        assert_eq!(group_thousands("-1234567"), "-1,234,567");
        assert_eq!(group_thousands("12"), "12");
        assert_eq!(group_thousands("999"), "999");
    }

    #[test]
    fn bound_walking_is_row_major_and_skips_nothing_it_should_not() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("B1", Value::Number(2.0)),
            ("A2", Value::Number(3.0)),
            ("B2", Value::Number(4.0)),
        ];
        let source = sheet(&cells);
        let bounds = RangeRef::new(CellRef::new(0, 0), CellRef::new(1, 1)).bounds().unwrap();
        let seen = cells_in_order(&source, bounds);
        assert_eq!(
            seen.iter().map(|(o, v)| (*o, v.as_number().unwrap())).collect::<Vec<_>>(),
            vec![(0, 1.0), (1, 2.0), (2, 3.0), (3, 4.0)]
        );
        assert_eq!(value_at_offset(&source, bounds, 1), Value::Number(2.0));
        assert_eq!(value_at_offset(&source, bounds, 4), Value::Empty);
    }

    #[test]
    fn whole_column_bounds_are_walked_sparsely() {
        let cells = [("A3", Value::Number(7.0)), ("A900000", Value::Number(8.0))];
        let source = sheet(&cells);
        let bounds = RangeRef::new(CellRef::new(0, 0), CellRef::new(crate::addr::MAX_ROWS - 1, 0)).bounds().unwrap();
        let mut seen = Vec::new();
        each_in_bounds(&source, bounds, &mut |cell, value| seen.push((cell.a1(), value.as_number().unwrap())));
        assert_eq!(seen, vec![("A3".to_string(), 7.0), ("A900000".to_string(), 8.0)]);
    }

    #[test]
    fn iferror_evaluates_its_value_exactly_once() {
        let failing = CountingSource::new(&[("A1", Value::Error(ErrorKind::Div0))]);
        assert_eq!(eval_with("=IFERROR(A1, 42)", &failing, "B1"), Value::Number(42.0));
        assert_eq!(failing.read_count("A1"), 1, "the failing value is read once");

        let succeeding = CountingSource::new(&[("A1", Value::Number(7.0))]);
        assert_eq!(eval_with("=IFERROR(A1, 42)", &succeeding, "B1"), Value::Number(7.0));
        assert_eq!(succeeding.read_count("A1"), 1, "and once when it succeeds");
    }

    #[test]
    fn ifna_reads_the_fallback_only_for_a_missing_value() {
        let missing = CountingSource::new(&[("A1", Value::Error(ErrorKind::NA)), ("A2", Value::Number(1.0))]);
        assert_eq!(eval_with("=IFNA(A1, A2)", &missing, "B1"), Value::Number(1.0));
        assert_eq!(missing.read_count("A2"), 1, "the fallback ran for #N/A");

        let other = CountingSource::new(&[("A1", Value::Error(ErrorKind::Div0)), ("A2", Value::Number(1.0))]);
        assert_eq!(eval_with("=IFNA(A1, A2)", &other, "B1"), Value::Error(ErrorKind::Div0));
        assert_eq!(other.read_count("A2"), 0, "the fallback was skipped");
    }

    #[test]
    fn ifs_does_not_evaluate_the_pairs_it_skips() {
        let source = CountingSource::new(&[("A1", Value::Bool(true)), ("A2", Value::Number(99.0))]);
        assert_eq!(eval_with("=IFS(A1, 1, A2, 2)", &source, "B1"), Value::Number(1.0));
        assert_eq!(source.read_count("A1"), 1);
        assert_eq!(source.read_count("A2"), 0, "the second condition was never asked");
    }
}
