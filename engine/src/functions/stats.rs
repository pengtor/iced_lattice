
use super::*;

pub(super) fn dispatch(
    func: FuncId,
    args: &[Operand],
    source: &dyn ValueSource,
    _host: CellRef,
) -> Option<Value> {
    Some(match func {
        FuncId::Median => median(args, source),
        FuncId::Mode => mode(args, source),
        FuncId::StDev => variance_like(args, source, true),
        FuncId::Var => variance_like(args, source, false),
        _ => return None,
    })
}

fn collect(args: &[Operand], source: &dyn ValueSource) -> Result<Vec<f64>, ErrorKind> {
    let mut numbers = Vec::new();
    collect_numbers(args, source, &mut numbers)?;
    Ok(numbers)
}

// collected numbers are finite, so partial_cmp cannot fail
fn sort(numbers: &mut [f64]) {
    numbers.sort_by(|a, b| a.partial_cmp(b).expect("collected numbers are finite"));
}

fn median(args: &[Operand], source: &dyn ValueSource) -> Value {
    let mut numbers = match collect(args, source) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    if numbers.is_empty() {
        return Value::Error(ErrorKind::Div0);
    }
    sort(&mut numbers);
    let mid = numbers.len() / 2;
    let value = if numbers.len() % 2 == 1 {
        numbers[mid]
    } else {
        (numbers[mid - 1] + numbers[mid]) / 2.0
    };
    Value::finite_number(value)
}

fn mode(args: &[Operand], source: &dyn ValueSource) -> Value {
    let mut numbers = match collect(args, source) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    if numbers.is_empty() {
        return Value::Error(ErrorKind::Div0);
    }
    sort(&mut numbers);

    // sorted ascending; strictly-greater wins, so ties go to smallest
    let mut best_count = 0usize;
    let mut best_value = numbers[0];
    let mut i = 0;
    while i < numbers.len() {
        let mut j = i + 1;
        while j < numbers.len() && numbers[j] == numbers[i] {
            j += 1;
        }
        if j - i > best_count {
            best_count = j - i;
            best_value = numbers[i];
        }
        i = j;
    }

    if best_count <= 1 {
        // nothing repeats: Excel answers #N/A, not the value
        Value::Error(ErrorKind::NA)
    } else {
        Value::finite_number(best_value)
    }
}

// two passes for stability; one-pass form cancels catastrophically
fn variance_like(args: &[Operand], source: &dyn ValueSource, stdev: bool) -> Value {
    let numbers = match collect(args, source) {
        Ok(numbers) => numbers,
        Err(kind) => return Value::Error(kind),
    };
    if numbers.len() < 2 {
        return Value::Error(ErrorKind::Div0);
    }
    let n = numbers.len() as f64;
    let mean = numbers.iter().sum::<f64>() / n;
    let sum_squared_deviations: f64 = numbers
        .iter()
        .map(|x| {
            let deviation = x - mean;
            deviation * deviation
        })
        .sum();
    let variance = sum_squared_deviations / (n - 1.0);
    if stdev {
        Value::finite_number(variance.sqrt())
    } else {
        Value::finite_number(variance)
    }
}

#[cfg(test)]
mod tests {
    use crate::functions::test_support::{assert_eq_value, assert_error, eval_at};
    use crate::{ErrorKind, Value};

    #[test]
    fn median_takes_the_middle_and_averages_the_two_middles() {
        assert_eq_value("=MEDIAN(3, 1, 2)", Value::Number(2.0));
        assert_eq_value("=MEDIAN(1, 2, 3, 4)", Value::Number(2.5));
        assert_eq_value("=MEDIAN(5, 5, 1, 9, 2)", Value::Number(5.0));
        assert_eq_value("=MEDIAN(7)", Value::Number(7.0));
    }

    #[test]
    fn median_over_a_range_ignores_text_and_booleans() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Text("n/a".into())),
            ("A3", Value::Number(10.0)),
            ("A4", Value::Bool(true)),
            ("A5", Value::Number(5.0)),
        ];
        assert_eq!(eval_at("=MEDIAN(A1:A5)", &cells, "B1"), Value::Number(5.0));
    }

    #[test]
    fn median_of_nothing_is_div_zero_and_text_is_a_type_error() {
        let empty = [("A1", Value::Text("n/a".into()))];
        assert_eq!(eval_at("=MEDIAN(A1:A1)", &empty, "B1"), Value::Error(ErrorKind::Div0));
        assert_error("=MEDIAN(\"x\")", ErrorKind::Value);
        let cells = [("A1", Value::Error(ErrorKind::Num))];
        assert_eq!(eval_at("=MEDIAN(A1:A1)", &cells, "B1"), Value::Error(ErrorKind::Num));
    }

    #[test]
    fn mode_picks_the_most_frequent_value() {
        assert_eq_value("=MODE(1, 2, 2, 3)", Value::Number(2.0));
        assert_eq_value("=MODE(4, 4, 4, 1, 1)", Value::Number(4.0));
        let cells = [
            ("A1", Value::Number(7.0)),
            ("A2", Value::Number(7.0)),
            ("A3", Value::Number(3.0)),
        ];
        assert_eq!(eval_at("=MODE(A1:A3)", &cells, "B1"), Value::Number(7.0));
    }

    #[test]
    fn mode_breaks_ties_toward_the_smallest_and_answers_na_when_nothing_repeats() {
        assert_eq_value("=MODE(2, 2, 1, 1)", Value::Number(1.0));
        assert_error("=MODE(1, 2, 3)", ErrorKind::NA);
        let empty = [("A1", Value::Text("n/a".into()))];
        assert_eq!(eval_at("=MODE(A1:A1)", &empty, "B1"), Value::Error(ErrorKind::Div0));
        assert_error("=MODE(\"x\")", ErrorKind::Value);
    }

    #[test]
    fn var_and_stdev_are_the_sample_statistics() {
        assert_eq_value("=VAR(1, 2, 3, 4, 5)", Value::Number(2.5));
        assert_eq_value("=STDEV(1, 2, 3, 4, 5)", Value::Number(2.5f64.sqrt()));
        assert_eq_value("=VAR(2.0, 4.0, 6.0)", Value::Number(4.0));
    }

    #[test]
    fn var_and_stdev_of_a_single_value_or_nothing_are_div_zero() {
        assert_error("=VAR(3)", ErrorKind::Div0);
        assert_error("=STDEV(3)", ErrorKind::Div0);
        let empty = [("A1", Value::Text("n/a".into()))];
        assert_eq!(eval_at("=VAR(A1:A1)", &empty, "B1"), Value::Error(ErrorKind::Div0));
        assert_eq!(eval_at("=STDEV(A1:A1)", &empty, "B1"), Value::Error(ErrorKind::Div0));
        assert_error("=STDEV(1, \"x\")", ErrorKind::Value);
    }

    #[test]
    fn two_pass_variance_keeps_precision_on_large_values_with_small_spread() {
        assert_eq_value("=VAR(1000000000, 1000000001, 1000000002)", Value::Number(1.0));
    }
}
