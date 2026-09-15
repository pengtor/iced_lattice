//! The evaluator: a stack machine over the postfix program produced by
//! [`crate::compile`].
//!
//! Evaluation never recurses into other formulas. By the time a cell is evaluated,
//! every cell it reads already holds its final value (the recalculation scheduler
//! guarantees that), so the evaluator only ever *reads* the sheet through a
//! [`ValueSource`]. That property is what makes circular references impossible to
//! loop on and parallel evaluation of independent cells safe.

use crate::addr::{CellRef, RangeRef};
use crate::ast::{BinOp, UnOp};
use crate::compile::{Op, Program};
use crate::error::ErrorKind;
use crate::functions;
use crate::value::Value;

/// Ranges no larger than this are walked cell by cell; larger ones are answered
/// from the sparse map instead. Above this size, scanning stored cells is cheaper
/// than touching every empty cell of the rectangle.
pub const DENSE_RANGE_LIMIT: u64 = 1024;

/// Read-only access to cell values, used by the evaluator and by the built-in
/// functions. Implemented by [`crate::Sheet`], and by `HashMap<CellRef, Value>` for
/// tests and standalone evaluation.
pub trait ValueSource {
    /// The value of a cell; [`Value::Empty`] when the cell is unused.
    fn value(&self, cell: CellRef) -> Value;

    /// Visit every stored (non-empty) cell. Used to iterate large ranges sparsely.
    fn each_stored(&self, visit: &mut dyn FnMut(CellRef, &Value));

    /// Visit the values inside `range` in row-major order, skipping empty cells.
    ///
    /// Empty cells are skipped because no built-in function in the v1 language
    /// distinguishes "empty" from "absent": `SUM`, `COUNT`, `MIN`, `MAX` ignore
    /// them and `CONCAT` appends nothing for them. Small rectangles are walked
    /// cell by cell; large ones are answered from the sparse map so that
    /// `SUM(A1:A100000)` does not touch a hundred thousand empty cells.
    fn visit_range(&self, range: RangeRef, visit: &mut dyn FnMut(Value)) {
        let Some(bounds) = range.bounds() else {
            return;
        };
        if bounds.len() <= DENSE_RANGE_LIMIT {
            for cell in bounds.iter_cells() {
                let value = self.value(cell);
                if !value.is_empty() {
                    visit(value);
                }
            }
        } else {
            self.each_stored(&mut |cell, value| {
                if bounds.contains(cell) {
                    visit(value.clone());
                }
            });
        }
    }
}

impl ValueSource for std::collections::HashMap<CellRef, Value> {
    fn value(&self, cell: CellRef) -> Value {
        self.get(&cell).cloned().unwrap_or(Value::Empty)
    }

    fn each_stored(&self, visit: &mut dyn FnMut(CellRef, &Value)) {
        for (cell, value) in self {
            visit(*cell, value);
        }
    }
}

/// A value on the operand stack: either a scalar or a whole range waiting to be
/// consumed by an aggregate function.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Value(Value),
    Range(RangeRef),
}

impl Operand {
    /// Force a range into a single value using implicit intersection.
    pub fn scalar(self, source: &dyn ValueSource, host: CellRef) -> Value {
        match self {
            Operand::Value(v) => v,
            Operand::Range(r) => implicit_intersection(r, host, source),
        }
    }
}

/// Evaluate a compiled formula in the context of `host` (the cell that owns it).
pub fn evaluate(program: &Program, source: &dyn ValueSource, host: CellRef) -> Value {
    let code = program.code();
    let consts = program.consts();
    let mut stack: Vec<Operand> = Vec::with_capacity(8);
    let mut ip: usize = 0;

    while ip < code.len() {
        let op = &code[ip];
        ip += 1;
        match op {
            Op::Const(index) => stack.push(Operand::Value(consts[*index as usize].clone())),
            Op::Ref(r) => {
                let value = match r.to_cell() {
                    Some(cell) => source.value(cell),
                    None => Value::Error(ErrorKind::Ref),
                };
                stack.push(Operand::Value(value));
            }
            Op::Range(range) => stack.push(Operand::Range(*range)),
            Op::Unary(op) => {
                let operand = pop(&mut stack).scalar(source, host);
                stack.push(Operand::Value(apply_unary(*op, operand)));
            }
            Op::Percent => {
                let operand = pop(&mut stack).scalar(source, host);
                let value = match operand.as_number() {
                    Ok(n) => Value::finite_number(n / 100.0),
                    Err(kind) => Value::Error(kind),
                };
                stack.push(Operand::Value(value));
            }
            Op::Binary(op) => {
                let rhs = pop(&mut stack).scalar(source, host);
                let lhs = pop(&mut stack).scalar(source, host);
                stack.push(Operand::Value(apply_binary(*op, lhs, rhs)));
            }
            Op::Call { func, argc } => {
                let args = pop_args(&mut stack, *argc);
                stack.push(Operand::Value(functions::dispatch(*func, &args, source)));
            }
            Op::JumpIfFalse(target) => {
                let condition = pop(&mut stack).scalar(source, host);
                if let Value::Error(kind) = condition {
                    // A failing condition propagates instead of choosing a branch.
                    return Value::Error(kind);
                }
                if !condition.as_bool().unwrap_or(false) {
                    ip = *target;
                }
            }
            Op::Jump(target) => ip = *target,
        }
    }

    pop(&mut stack).scalar(source, host)
}

fn pop(stack: &mut Vec<Operand>) -> Operand {
    stack.pop().unwrap_or(Operand::Value(Value::Empty))
}

/// Pop `argc` operands, restoring their source order.
fn pop_args(stack: &mut Vec<Operand>, argc: usize) -> Vec<Operand> {
    let mut args = Vec::with_capacity(argc);
    for _ in 0..argc {
        args.push(pop(stack));
    }
    args.reverse();
    args
}

/// Resolve a range in a scalar context.
///
/// Spreadsheets call this *implicit intersection*: a range used where a single
/// value is expected collapses to the cell that lines up with the formula — the
/// formula's row for a one-column range, the formula's column for a one-row range,
/// and the formula's own cell for a rectangular range that contains it. If nothing
/// lines up, the result is `#VALUE!`.
pub fn implicit_intersection(range: RangeRef, host: CellRef, source: &dyn ValueSource) -> Value {
    let Some(bounds) = range.bounds() else {
        return Value::Error(ErrorKind::Ref);
    };
    let single_column = bounds.cols() == 1;
    let single_row = bounds.rows() == 1;

    let cell = if single_column && single_row {
        CellRef::new(bounds.min_row, bounds.min_col)
    } else if single_column {
        if host.row < bounds.min_row || host.row > bounds.max_row {
            return Value::Error(ErrorKind::Value);
        }
        CellRef::new(host.row, bounds.min_col)
    } else if single_row {
        if host.col < bounds.min_col || host.col > bounds.max_col {
            return Value::Error(ErrorKind::Value);
        }
        CellRef::new(bounds.min_row, host.col)
    } else if bounds.contains(host) {
        host
    } else {
        return Value::Error(ErrorKind::Value);
    };

    source.value(cell)
}

/// Apply a binary operator, propagating errors before doing any arithmetic.
pub fn apply_binary(op: BinOp, lhs: Value, rhs: Value) -> Value {
    if let Value::Error(kind) = lhs {
        return Value::Error(kind);
    }
    if let Value::Error(kind) = rhs {
        return Value::Error(kind);
    }

    if op.is_comparison() {
        return match lhs.compare(&rhs) {
            Ok(ordering) => {
                use std::cmp::Ordering::{Equal, Greater, Less};
                let result = match op {
                    BinOp::Eq => ordering == Equal,
                    BinOp::Ne => ordering != Equal,
                    BinOp::Lt => ordering == Less,
                    BinOp::Le => ordering != Greater,
                    BinOp::Gt => ordering == Greater,
                    BinOp::Ge => ordering != Less,
                    _ => unreachable!("is_comparison guaranteed a comparison"),
                };
                Value::Bool(result)
            }
            Err(kind) => Value::Error(kind),
        };
    }

    let a = match lhs.as_number() {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    let b = match rhs.as_number() {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };

    let result = match op {
        BinOp::Add => a + b,
        BinOp::Sub => a - b,
        BinOp::Mul => a * b,
        BinOp::Div => {
            if b == 0.0 {
                return Value::Error(ErrorKind::Div0);
            }
            a / b
        }
        // `(-8)^0.5` is not a real number, and `1e308*10` overflows: both become
        // `#NUM!` rather than leaking `NaN`/`inf` into the sheet.
        BinOp::Pow => a.powf(b),
        _ => unreachable!("comparisons handled above"),
    };
    Value::finite_number(result)
}

/// Apply a unary operator, propagating errors.
pub fn apply_unary(op: UnOp, operand: Value) -> Value {
    match operand.as_number() {
        Ok(n) => match op {
            UnOp::Neg => Value::finite_number(-n),
            UnOp::Plus => Value::finite_number(n),
        },
        Err(kind) => Value::Error(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addr::Ref;
    use crate::compile::compile_source;
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
        let host = CellRef::parse_a1(host).unwrap();
        evaluate(&program, &source, host)
    }

    fn eval(src: &str) -> Value {
        eval_at(src, &[], "A1")
    }

    #[test]
    fn arithmetic_follows_spreadsheet_precedence() {
        assert_eq!(eval("=1+2*3"), Value::Number(7.0));
        assert_eq!(eval("=(1+2)*3"), Value::Number(9.0));
        assert_eq!(eval("=10-2-3"), Value::Number(5.0));
        assert_eq!(eval("=2^3^2"), Value::Number(64.0)); // left-associative
        assert_eq!(eval("=-2^2"), Value::Number(4.0)); // unary binds tighter
        assert_eq!(eval("=50%"), Value::Number(0.5));
        assert_eq!(eval("=1+50%"), Value::Number(1.5));
    }

    #[test]
    fn division_by_zero_is_an_error_value() {
        assert_eq!(eval("=1/0"), Value::Error(ErrorKind::Div0));
        assert_eq!(eval("=0/0"), Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn non_finite_results_become_num_errors() {
        assert_eq!(eval("=(-8)^0.5"), Value::Error(ErrorKind::Num));
        assert_eq!(eval("=1e308*10"), Value::Error(ErrorKind::Num));
    }

    #[test]
    fn errors_propagate_through_operators() {
        assert_eq!(eval("=#N/A+1"), Value::Error(ErrorKind::NA));
        assert_eq!(eval("=1+#REF!"), Value::Error(ErrorKind::Ref));
        assert_eq!(eval("=-#DIV/0!"), Value::Error(ErrorKind::Div0));
    }

    #[test]
    fn references_read_the_sheet() {
        let cells = [("A1", Value::Number(2.0)), ("B1", Value::Number(5.0))];
        assert_eq!(eval_at("=A1*B1", &cells, "C1"), Value::Number(10.0));
        assert_eq!(eval_at("=A1+Z9", &cells, "C1"), Value::Number(2.0));
    }

    #[test]
    fn text_is_not_silently_numeric() {
        let cells = [("A1", Value::Text("5".into()))];
        assert_eq!(eval_at("=A1+1", &cells, "B1"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn comparisons_produce_booleans() {
        assert_eq!(eval("=1<2"), Value::Bool(true));
        assert_eq!(eval("=1<>1"), Value::Bool(false));
        assert_eq!(eval("=\"a\"<\"b\""), Value::Bool(true));
        assert_eq!(eval("=1<2<3"), Value::Bool(false)); // (1<2) is TRUE, TRUE > 3
    }

    #[test]
    fn ranges_in_scalar_context_use_implicit_intersection() {
        let cells = [("A1", Value::Number(1.0)), ("A2", Value::Number(2.0)), ("A3", Value::Number(3.0))];
        // One column: the formula's row decides.
        assert_eq!(eval_at("=A1:A3+10", &cells, "B2"), Value::Number(12.0));
        // Outside the column's rows: no intersection.
        assert_eq!(eval_at("=A1:A3+10", &cells, "B9"), Value::Error(ErrorKind::Value));
        // A rectangle only intersects when the formula is inside it.
        assert_eq!(eval_at("=A1:B2+1", &cells, "B2"), Value::Number(1.0));
        assert_eq!(eval_at("=A1:B2+1", &cells, "D4"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn lazy_if_does_not_evaluate_the_untaken_branch() {
        // The classic case: the else branch divides by zero, but is never run.
        assert_eq!(eval_at("=IF(A1=0, 0, 1/A1)", &[("A1", Value::Number(0.0))], "B1"), Value::Number(0.0));
        assert_eq!(eval_at("=IF(A1=0, 0, 1/A1)", &[("A1", Value::Number(4.0))], "B1"), Value::Number(0.25));
        assert_eq!(eval("=IF(FALSE, 1)"), Value::Bool(false));
        assert_eq!(eval("=IF(TRUE, 1)"), Value::Number(1.0));
    }

    #[test]
    fn a_failing_condition_propagates_instead_of_branching() {
        assert_eq!(eval("=IF(#N/A, 1, 2)"), Value::Error(ErrorKind::NA));
    }

    #[test]
    fn off_sheet_references_evaluate_to_ref_errors() {
        let program = compile_source("=A1").unwrap();
        // Rewrite the reference to point off-sheet, as a fill would.
        assert_eq!(Ref::parse("A1").unwrap().shifted(-1, 0).to_cell(), None);
        let text = crate::ast::Expr::Error(ErrorKind::Ref, 0..0).to_string();
        let program_for_error = compile_source(&format!("={text}")).unwrap();
        let source: HashMap<CellRef, Value> = HashMap::new();
        assert_eq!(evaluate(&program_for_error, &source, CellRef::new(0, 0)), Value::Error(ErrorKind::Ref));
        let _ = program;
    }

    #[test]
    fn unary_minus_on_booleans_and_empty() {
        assert_eq!(eval("=-TRUE"), Value::Number(-1.0));
        assert_eq!(eval("=Z99+1"), Value::Number(1.0));
    }
}
