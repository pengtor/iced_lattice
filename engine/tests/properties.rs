
use std::collections::HashMap;

use proptest::prelude::*;

use engine::ast::{Expr, UnOp};
use engine::compile::{compile, compile_source};
use engine::eval::evaluate;
use engine::{
    BinOp, CellRef, ErrorKind, RangeRef, Ref, Sheet, Value,
};

// names that survive a print/reparse round trip as names
const SAFE_NAMES: [&str; 4] = ["TOTAL", "MY_NAME", "X", "grand_total"];

fn cell_ref_strategy() -> impl Strategy<Value = Ref> {
    (0i64..30, 0i64..10, any::<bool>(), any::<bool>()).prop_map(|(row, col, row_abs, col_abs)| {
        Ref { row, col, row_abs, col_abs }
    })
}

fn leaf_strategy() -> impl Strategy<Value = Expr> {
    prop_oneof![
        // negatives print as unary minus, changing the tree's shape
        (0.0f64..1e6).prop_map(|n| Expr::Number(n, 0..0)),
        prop::sample::select(SAFE_NAMES.to_vec()).prop_map(|n| Expr::Name(n.to_string(), 0..0)),
        any::<bool>().prop_map(|b| Expr::Bool(b, 0..0)),
        prop::sample::select(vec![
            ErrorKind::Div0,
            ErrorKind::NA,
            ErrorKind::Ref,
            ErrorKind::Value,
            ErrorKind::Name,
            ErrorKind::Num,
            ErrorKind::Cycle,
            ErrorKind::Parse,
        ])
        .prop_map(|k| Expr::Error(k, 0..0)),
        cell_ref_strategy().prop_map(|r| Expr::Ref(r, 0..0)),
        (cell_ref_strategy(), cell_ref_strategy())
            .prop_map(|(start, end)| Expr::Range(RangeRef { start, end }, 0..0)),
        "[a-zA-Z0-9 ]{0,8}".prop_map(|s| Expr::Text(s, 0..0)),
    ]
}

fn any_expr() -> impl Strategy<Value = Expr> {
    leaf_strategy().prop_recursive(4, 48, 4, |inner| {
        prop_oneof![
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| binary(BinOp::Add, a, b)),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| binary(BinOp::Sub, a, b)),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| binary(BinOp::Mul, a, b)),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| binary(BinOp::Div, a, b)),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| binary(BinOp::Pow, a, b)),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| binary(BinOp::Lt, a, b)),
            (inner.clone(), inner.clone())
                .prop_map(|(a, b)| binary(BinOp::Eq, a, b)),
            inner.clone().prop_map(|e| Expr::Unary {
                op: UnOp::Neg,
                operand: Box::new(e),
                span: 0..0
            }),
            inner.clone().prop_map(|e| Expr::Percent {
                operand: Box::new(e),
                span: 0..0
            }),
            (prop::sample::select(vec!["SUM", "NOPE", "CONCAT"]), prop::collection::vec(inner.clone(), 0..3))
                .prop_map(|(name, args)| Expr::Call {
                    name: name.to_string(),
                    args,
                    span: 0..0
                }),
            (inner.clone(), inner.clone(), inner.clone())
                .prop_map(|(c, a, b)| Expr::Call { name: "IF".into(), args: vec![c, a, b], span: 0..0 }),
        ]
    })
}

fn numeric_expr() -> impl Strategy<Value = Expr> {
    (1.0f64..100.0).prop_map(|n| Expr::Number(n, 0..0)).prop_recursive(
        4,
        24,
        2,
        |inner| {
            prop_oneof![
                (inner.clone(), inner.clone()).prop_map(|(a, b)| binary(BinOp::Add, a, b)),
                (inner.clone(), inner.clone()).prop_map(|(a, b)| binary(BinOp::Sub, a, b)),
                (inner.clone(), inner.clone()).prop_map(|(a, b)| binary(BinOp::Mul, a, b)),
                (inner.clone(), inner.clone()).prop_map(|(a, b)| binary(BinOp::Div, a, b)),
                (inner.clone(), inner.clone()).prop_map(|(a, b)| binary(BinOp::Pow, a, b)),
                inner.clone().prop_map(|e| Expr::Unary {
                    op: UnOp::Neg,
                    operand: Box::new(e),
                    span: 0..0
                }),
            ]
        },
    )
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span: 0..0 }
}

// reference tree evaluator; None means the engine should error
fn eval_tree(expr: &Expr) -> Option<f64> {
    fn finite(n: f64) -> Option<f64> {
        n.is_finite().then_some(n)
    }
    match expr {
        Expr::Number(n, _) => Some(*n),
        Expr::Unary { op, operand, .. } => {
            let value = eval_tree(operand)?;
            match op {
                UnOp::Neg => finite(-value),
                UnOp::Plus => finite(value),
            }
        }
        Expr::Percent { operand, .. } => finite(eval_tree(operand)? / 100.0),
        Expr::Binary { op, lhs, rhs, .. } => {
            let a = eval_tree(lhs)?;
            let b = eval_tree(rhs)?;
            match op {
                BinOp::Add => finite(a + b),
                BinOp::Sub => finite(a - b),
                BinOp::Mul => finite(a * b),
                BinOp::Div => {
                    if b == 0.0 {
                        None
                    } else {
                        finite(a / b)
                    }
                }
                BinOp::Pow => finite(a.powf(b)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn empty_source() -> HashMap<CellRef, Value> {
    HashMap::new()
}

proptest! {
    #[test]
    fn printing_then_parsing_round_trips(expr in any_expr()) {
        let text = expr.to_string();
        let parsed = engine::parse(&text)
            .unwrap_or_else(|e| panic!("printed {text:?} failed to parse: {e}"));
        prop_assert!(expr.same_shape(&parsed), "{} printed as {text}", "round trip changed the tree");
    }

    // fill/copy relies on shift staying printable and shape-preserving
    #[test]
    fn shifting_is_shape_preserving_and_printable(
        expr in any_expr(),
        row_delta in -40i64..40,
        col_delta in -40i64..40,
    ) {
        let shifted = expr.shifted(row_delta, col_delta);
        prop_assert_eq!(shifted.size(), expr.size());
        let text = shifted.to_string();
        let reparsed = engine::parse(&text)
            .unwrap_or_else(|e| panic!("shifted formula {text:?} failed to parse: {e}"));
        prop_assert!(shifted.same_shape(&reparsed));
    }

    #[test]
    fn shifting_by_zero_changes_nothing(expr in any_expr()) {
        prop_assert!(expr.same_shape(&expr.shifted(0, 0)));
    }

    #[test]
    fn parsing_arbitrary_text_is_safe(text in ".{0,40}") {
        match engine::parse(&text) {
            Ok(_) => {}
            Err(diagnostic) => {
                prop_assert!(diagnostic.span.0 <= text.len());
                prop_assert!(diagnostic.span.1 <= text.len());
                prop_assert!(!diagnostic.message.is_empty());
            }
        }
    }

    #[test]
    fn bytecode_evaluation_matches_tree_evaluation(expr in numeric_expr()) {
        let program = compile(&expr).expect("numeric expressions always compile");
        let source = empty_source();
        let host = CellRef::new(0, 0);
        let actual = evaluate(&program, &source, host);
        let expected = eval_tree(&expr);
        match (actual, expected) {
            (Value::Number(actual), Some(expected)) => prop_assert_eq!(actual, expected),
            (Value::Error(_), None) => {}
            (actual, expected) => prop_assert!(
                false,
                "engine said {actual:?} but the reference evaluator said {expected:?} for {expr}"
            ),
        }
    }

    #[test]
    fn values_are_never_nan_or_infinite(expr in numeric_expr()) {
        let program = compile(&expr).expect("compiles");
        let source = empty_source();
        if let Value::Number(n) = evaluate(&program, &source, CellRef::new(0, 0)) {
            prop_assert!(n.is_finite());
        }
    }

    #[test]
    fn range_aggregation_matches_explicit_addition(values in prop::collection::vec(-1000.0f64..1000.0, 1..9)) {
        let mut sheet = Sheet::new();
        for (index, value) in values.iter().enumerate() {
            sheet.set_value(CellRef::new(index as u32, 0), Value::Number(*value));
        }
        let last_row = values.len() as u32;
        let range_sum = sheet.set_input(CellRef::new(0, 1), &format!("=SUM(A1:A{last_row})"));
        prop_assert!(range_sum.cycles.is_empty());

        let explicit = format!("={}", (1..=last_row).map(|r| format!("A{r}")).collect::<Vec<_>>().join("+"));
        sheet.set_input(CellRef::new(0, 2), &explicit);

        let expected: f64 = values.iter().sum();
        prop_assert_eq!(sheet.value(CellRef::new(0, 1)), Value::Number(expected));
        let explicit_value = match sheet.value(CellRef::new(0, 2)) {
            Value::Number(n) => n,
            other => {
                prop_assert!(false, "explicit sum was {other:?}");
                unreachable!()
            }
        };
        // Both addition orders must agree exactly at these magnitudes
        prop_assert_eq!(explicit_value, expected);
    }

    #[test]
    fn recalculation_is_idempotent_and_values_stay_finite(
        ops in prop::collection::vec((0u32..3, 0u32..3, 0u32..6), 1..25)
    ) {
        const INPUTS: [&str; 6] = ["1", "2.5", "=A1+B1", "=SUM(A1:C3)", "=IF(A1>2, B1, C1)", "=A1*2"];
        let mut sheet = Sheet::new();
        let mut log: Vec<String> = Vec::new();
        for (row, col, choice) in &ops {
            let cell = CellRef::new(*row, *col);
            log.push(format!("{} = {}", cell.a1(), INPUTS[*choice as usize]));
            sheet.set_input(cell, INPUTS[*choice as usize]);
        }

        let before: Vec<(CellRef, Value)> = sheet.iter_cells().map(|(c, s)| (c, s.value.clone())).collect();
        for (cell, value) in &before {
            if let Value::Number(n) = value {
                prop_assert!(n.is_finite(), "{} held {n}", cell.a1());
            }
        }

        sheet.recalculate_all();
        for (cell, value) in &before {
            prop_assert_eq!(&sheet.value(*cell), value, "recalculation changed {} (edits: {:?})", cell.a1(), log);
        }
    }

    #[test]
    fn cycles_are_reported_and_recoverable(rows in 2usize..12) {
        let mut sheet = Sheet::new();
        for row in 0..rows {
            let next = (row + 1) % rows;
            sheet.set_input(CellRef::new(row as u32, 0), &format!("=A{}+1", next + 1));
        }
        let report = sheet.recalculate_all();
        prop_assert_eq!(report.cycles.len(), rows);
        for row in 0..rows {
            prop_assert_eq!(sheet.value(CellRef::new(row as u32, 0)), Value::Error(ErrorKind::Cycle));
        }

        sheet.set_input(CellRef::new(0, 0), "0");
        for row in 0..rows {
            prop_assert!(
                matches!(sheet.value(CellRef::new(row as u32, 0)), Value::Number(_)),
                "cell {} did not recover",
                row
            );
        }
    }

    #[test]
    fn evaluation_is_order_independent(
        values in prop::collection::vec(-100.0f64..100.0, 3..7)
    ) {
        let mut forward = Sheet::new();
        for (index, value) in values.iter().enumerate() {
            forward.set_value(CellRef::new(index as u32, 0), Value::Number(*value));
        }
        forward.set_formula(CellRef::new(0, 1), "=SUM(A1:A6)+A1*A2");

        let mut backward = Sheet::new();
        backward.set_formula(CellRef::new(0, 1), "=SUM(A1:A6)+A1*A2");
        for (index, value) in values.iter().enumerate().rev() {
            backward.set_value(CellRef::new(index as u32, 0), Value::Number(*value));
        }

        prop_assert_eq!(forward.value(CellRef::new(0, 1)), backward.value(CellRef::new(0, 1)));
    }

    #[test]
    fn precedents_cover_every_reference(expr in any_expr()) {
        let Ok(program) = compile(&expr) else { return Ok(()); };
        let precedents = program.precedents();
        // off-sheet refs yield #REF! and need no precedent entry
        for op in program.code() {
            if let engine::Op::Ref(r) = op {
                if let Some(cell) = r.to_cell() {
                    prop_assert!(precedents.contains(&engine::Precedent::Cell(cell)));
                }
            }
        }
    }

    #[test]
    fn compile_source_agrees_with_parse_then_compile(expr in any_expr()) {
        let text = expr.to_string();
        let direct = compile_source(&text);
        let staged = engine::parse(&text).and_then(|parsed| compile(&parsed));
        match (direct, staged) {
            (Ok(a), Ok(b)) => prop_assert_eq!(a, b),
            (Err(a), Err(b)) => prop_assert_eq!(a.message, b.message),
            (a, b) => prop_assert!(false, "{a:?} vs {b:?}"),
        }
    }
}
