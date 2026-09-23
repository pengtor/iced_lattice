use super::*;

#[derive(Clone, Copy)]
enum Fold {
    And,
    Or,
    Xor,
}

pub(super) fn dispatch(
    func: FuncId,
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
) -> Option<Value> {
    Some(match func {
        FuncId::And => logic_fold(args, source, Fold::And),
        FuncId::Or => logic_fold(args, source, Fold::Or),
        FuncId::Xor => logic_fold(args, source, Fold::Xor),
        FuncId::Not => not(args, source, host),
        FuncId::IsBlank => type_predicate(args, source, host, |value| matches!(value, Value::Empty)),
        FuncId::IsNumber => type_predicate(args, source, host, |value| matches!(value, Value::Number(_))),
        FuncId::IsText => type_predicate(args, source, host, |value| matches!(value, Value::Text(_))),
        FuncId::IsError => type_predicate(args, source, host, |value| matches!(value, Value::Error(_))),
        FuncId::IsNa => {
            type_predicate(args, source, host, |value| matches!(value, Value::Error(ErrorKind::NA)))
        }
        _ => return None,
    })
}

fn logic_fold(args: &[Operand], source: &dyn ValueSource, fold: Fold) -> Value {
    let values = match collect_bools(args, source) {
        Ok(values) => values,
        Err(kind) => return Value::Error(kind),
    };
    if values.is_empty() {
        // all-text/all-blank range: #VALUE!, not a vacuous TRUE
        return Value::Error(ErrorKind::Value);
    }
    let result = match fold {
        Fold::And => values.iter().all(|value| *value),
        Fold::Or => values.iter().any(|value| *value),
        Fold::Xor => values.iter().filter(|value| **value).count() % 2 == 1,
    };
    Value::Bool(result)
}

// ranges: numbers as n != 0, booleans kept, text/blank skipped
fn collect_bools(args: &[Operand], source: &dyn ValueSource) -> Result<Vec<bool>, ErrorKind> {
    let mut out = Vec::new();
    for arg in args {
        match arg {
            Operand::Value(value) => out.push(value.as_bool()?),
            Operand::Range(range) => {
                if range.bounds().is_none() {
                    return Err(ErrorKind::Ref);
                }
                let mut error = None;
                let mut values = Vec::new();
                source.visit_range(*range, &mut |value| match value {
                    Value::Number(n) => values.push(n != 0.0),
                    Value::Bool(b) => values.push(b),
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
    Ok(out)
}

fn not(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    match bool_arg(args, 0, source, host) {
        Ok(value) => Value::Bool(!value),
        Err(kind) => Value::Error(kind),
    }
}

// error args are passed to the predicate, not returned
fn type_predicate(
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    Value::Bool(predicate(&value_arg(args, 0, source, host)))
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use crate::{ErrorKind, Value};

    #[test]
    fn and_or_collect_booleans_from_scalars_and_ranges() {
        assert_eq!(eval("=AND(TRUE, 1, 2)"), Value::Bool(true));
        assert_eq!(eval("=AND(TRUE, 0)"), Value::Bool(false));
        assert_eq!(eval("=OR(FALSE, 0, 2)"), Value::Bool(true));
        assert_eq!(eval("=OR(0, 0)"), Value::Bool(false));

        let cells = [("A1", Value::Number(1.0)), ("A2", Value::Number(2.0))];
        assert_eq!(eval_at("=AND(A1:A2)", &cells, "B1"), Value::Bool(true));
        let cells = [("A1", Value::Number(1.0)), ("A2", Value::Number(0.0))];
        assert_eq!(eval_at("=OR(A1:A2)", &cells, "B1"), Value::Bool(true));
    }

    #[test]
    fn logic_ignores_text_inside_ranges_but_rejects_it_as_a_scalar() {
        let cells = [
            ("A1", Value::Number(1.0)),
            ("A2", Value::Text("n/a".into())),
            ("A3", Value::Number(2.0)),
        ];
        assert_eq!(eval_at("=AND(A1:A3)", &cells, "B1"), Value::Bool(true));
        assert_eq!(eval("=AND(\"n/a\")"), Value::Error(ErrorKind::Value));
        assert_eq!(eval("=OR(\"n/a\")"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn folding_an_all_text_range_is_a_value_error() {
        let cells = [("A1", Value::Text("x".into())), ("A2", Value::Text("y".into()))];
        assert_eq!(eval_at("=AND(A1:A2)", &cells, "B1"), Value::Error(ErrorKind::Value));
        assert_eq!(eval_at("=OR(A1:A2)", &cells, "B1"), Value::Error(ErrorKind::Value));
        assert_eq!(eval_at("=XOR(A1:A2)", &cells, "B1"), Value::Error(ErrorKind::Value));
        assert_eq!(eval_at("=AND(A1:A3)", &[], "B1"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn logic_propagates_the_first_error() {
        assert_eq!(eval("=AND(#N/A)"), Value::Error(ErrorKind::NA));
        assert_eq!(eval("=OR(#N/A, TRUE)"), Value::Error(ErrorKind::NA));
        let cells = [("A1", Value::Error(ErrorKind::Div0)), ("A2", Value::Number(1.0))];
        assert_eq!(eval_at("=AND(A1:A2)", &cells, "B1"), Value::Error(ErrorKind::Div0));
        assert_eq!(eval_at("=XOR(A1:A2)", &cells, "B1"), Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn xor_is_true_for_an_odd_count() {
        assert_eq!(eval("=XOR(TRUE, TRUE, TRUE)"), Value::Bool(true));
        assert_eq!(eval("=XOR(TRUE, TRUE)"), Value::Bool(false));
        assert_eq!(eval("=XOR(1, 0, 0)"), Value::Bool(true));
        assert_eq!(eval("=XOR(1, 1, 1, 1)"), Value::Bool(false));
    }

    #[test]
    fn not_negates_its_single_argument() {
        assert_eq!(eval("=NOT(0)"), Value::Bool(true));
        assert_eq!(eval("=NOT(TRUE)"), Value::Bool(false));
        assert_eq!(eval("=NOT(5)"), Value::Bool(false));
        assert_eq!(eval("=NOT(\"x\")"), Value::Error(ErrorKind::Value));
        assert_eq!(eval("=NOT(#N/A)"), Value::Error(ErrorKind::NA));
    }

    #[test]
    fn is_predicates_report_on_the_type_of_their_argument() {
        assert_eq!(eval("=ISNUMBER(1)"), Value::Bool(true));
        assert_eq!(eval("=ISNUMBER(\"1\")"), Value::Bool(false));
        assert_eq!(eval("=ISTEXT(\"a\")"), Value::Bool(true));
        assert_eq!(eval("=ISTEXT(1)"), Value::Bool(false));
        assert_eq!(eval_at("=ISBLANK(A1)", &[], "B1"), Value::Bool(true));
        let cells = [("A1", Value::Number(0.0))];
        assert_eq!(eval_at("=ISBLANK(A1)", &cells, "B1"), Value::Bool(false));
        assert_eq!(eval("=ISBLANK(\"\")"), Value::Bool(false));
    }

    #[test]
    fn error_predicates_look_at_the_error_instead_of_returning_it() {
        assert_eq!(eval("=ISERROR(#N/A)"), Value::Bool(true));
        assert_eq!(eval("=ISERROR(#DIV/0!)"), Value::Bool(true));
        assert_eq!(eval("=ISERROR(1)"), Value::Bool(false));
        assert_eq!(eval("=ISNA(#N/A)"), Value::Bool(true));
        assert_eq!(eval("=ISNA(#DIV/0!)"), Value::Bool(false));
        assert_eq!(eval("=ISNA(1)"), Value::Bool(false));
    }

    #[test]
    fn predicates_intersect_a_range_to_its_scalar() {
        let cells = [("A1", Value::Number(5.0))];
        assert_eq!(eval_at("=ISNUMBER(A1:A1)", &cells, "B1"), Value::Bool(true));
        assert_eq!(eval_at("=ISBLANK(A1:A1)", &[], "B1"), Value::Bool(true));
        assert_eq!(eval_at("=ISTEXT(A1:A1)", &cells, "B1"), Value::Bool(false));
    }
}
