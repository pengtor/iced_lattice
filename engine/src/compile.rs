//! AST → flat postfix (RPN) code, and the evaluator that runs it.
//!
//! # Why a bytecode
//!
//! A tree-walking evaluator would have to re-walk a cell's AST on every
//! recalculation. Compiling once to a flat, jump-based instruction sequence means
//! evaluation is a tight loop over a vector, and it lets `IF` be *lazy* —
//! `IF(A1=0, 0, 1/A1)` must not divide by zero, because the untaken branch is never
//! evaluated. Laziness is expressed with [`Op::JumpIfFalse`] / [`Op::Jump`].
//!
//! Function calls other than `IF` are strict; `IF` is the only short-circuiting
//! form in the v1 language.

use crate::ast::{BinOp, Expr, Span, UnOp};
use crate::error::{Diagnostic, ErrorKind};
use crate::value::Value;

/// Identifier of a built-in function (case-insensitive at the source level).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FuncId {
    Sum,
    Average,
    Count,
    If,
    Min,
    Max,
    Concat,
}

impl FuncId {
    /// Look up a function by name (case-insensitive).
    pub fn from_name(name: &str) -> Option<FuncId> {
        Some(match name.to_ascii_uppercase().as_str() {
            "SUM" => FuncId::Sum,
            "AVERAGE" => FuncId::Average,
            "COUNT" => FuncId::Count,
            "IF" => FuncId::If,
            "MIN" => FuncId::Min,
            "MAX" => FuncId::Max,
            "CONCAT" => FuncId::Concat,
            _ => return None,
        })
    }

    /// Canonical (upper-case) name.
    pub const fn name(self) -> &'static str {
        match self {
            FuncId::Sum => "SUM",
            FuncId::Average => "AVERAGE",
            FuncId::Count => "COUNT",
            FuncId::If => "IF",
            FuncId::Min => "MIN",
            FuncId::Max => "MAX",
            FuncId::Concat => "CONCAT",
        }
    }

    /// The argument counts this function accepts, as a human readable rule.
    pub const fn arity_rule(self) -> &'static str {
        match self {
            FuncId::If => "2 or 3 arguments",
            _ => "any number of arguments",
        }
    }

    fn accepts(self, argc: usize) -> bool {
        match self {
            FuncId::If => argc == 2 || argc == 3,
            _ => true,
        }
    }
}

impl std::fmt::Display for FuncId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A single instruction. Operands are pushed on and popped from the value stack.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Push a constant from the program's constant pool.
    Const(u32),
    /// Push the value of a cell.
    Ref(crate::addr::Ref),
    /// Push a reference to a whole range (consumed by aggregate functions).
    Range(crate::addr::RangeRef),
    /// Pop one operand, apply a unary operator, push the result.
    Unary(UnOp),
    /// Pop one operand, divide by 100, push the result.
    Percent,
    /// Pop two operands (lhs then rhs), apply a binary operator, push the result.
    Binary(BinOp),
    /// Pop `argc` operands and call a built-in.
    Call { func: FuncId, argc: usize },
    /// Pop a condition; if it is false, continue at the given index.
    JumpIfFalse(usize),
    /// Continue at the given index.
    Jump(usize),
}

/// A compiled formula: instruction sequence plus its constant pool.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    code: Vec<Op>,
    consts: Vec<Value>,
}

/// Something a formula reads: either a single cell or a whole rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precedent {
    /// A reference that resolved to a real cell.
    Cell(crate::addr::CellRef),
    /// A rectangular reference, kept as a rectangle so that huge ranges do not
    /// have to be expanded into individual edges.
    Range(crate::addr::RangeRef),
}

impl Program {
    pub fn code(&self) -> &[Op] {
        &self.code
    }

    pub fn consts(&self) -> &[Value] {
        &self.consts
    }

    /// Number of instructions (used by tests and diagnostics).
    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    /// The cells and ranges this program reads, in source order.
    ///
    /// This is what the dependency graph is built from: references that fell off
    /// the sheet are skipped (they contribute an error, not a dependency).
    pub fn precedents(&self) -> Vec<Precedent> {
        let mut out = Vec::new();
        for op in &self.code {
            match op {
                Op::Ref(r) => {
                    if let Some(cell) = r.to_cell() {
                        let precedent = Precedent::Cell(cell);
                        if !out.contains(&precedent) {
                            out.push(precedent);
                        }
                    }
                }
                Op::Range(r) => {
                    let precedent = Precedent::Range(*r);
                    if !out.contains(&precedent) {
                        out.push(precedent);
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// Compile a parsed expression into an evaluable program.
pub fn compile(expr: &Expr) -> Result<Program, Diagnostic> {
    let mut compiler = Compiler { code: Vec::new(), consts: Vec::new() };
    compiler.emit(expr)?;
    Ok(Program { code: compiler.code, consts: compiler.consts })
}

/// Parse and compile in one step.
pub fn compile_source(src: &str) -> Result<Program, Diagnostic> {
    let expr = crate::parser::parse(src)?;
    compile(&expr)
}

struct Compiler {
    code: Vec<Op>,
    consts: Vec<Value>,
}

impl Compiler {
    fn constant(&mut self, value: Value) -> u32 {
        // Reuse an existing slot when possible: formulas repeat literals often.
        if let Some(index) = self.consts.iter().position(|v| *v == value) {
            return index as u32;
        }
        self.consts.push(value);
        (self.consts.len() - 1) as u32
    }

    fn push(&mut self, op: Op) -> usize {
        self.code.push(op);
        self.code.len() - 1
    }

    fn emit(&mut self, expr: &Expr) -> Result<(), Diagnostic> {
        match expr {
            Expr::Number(n, _) => {
                let index = self.constant(Value::Number(*n));
                self.push(Op::Const(index));
            }
            Expr::Text(s, _) => {
                let index = self.constant(Value::Text(s.clone()));
                self.push(Op::Const(index));
            }
            Expr::Bool(b, _) => {
                let index = self.constant(Value::Bool(*b));
                self.push(Op::Const(index));
            }
            Expr::Error(kind, _) => {
                let index = self.constant(Value::Error(*kind));
                self.push(Op::Const(index));
            }
            Expr::Name(_, _) => {
                // An unrecognised name has no value: it is `#NAME?`.
                let index = self.constant(Value::Error(ErrorKind::Name));
                self.push(Op::Const(index));
            }
            Expr::Ref(r, _) => {
                self.push(Op::Ref(*r));
            }
            Expr::Range(r, _) => {
                self.push(Op::Range(*r));
            }
            Expr::Unary { op, operand, .. } => {
                self.emit(operand)?;
                self.push(Op::Unary(*op));
            }
            Expr::Percent { operand, .. } => {
                self.emit(operand)?;
                self.push(Op::Percent);
            }
            Expr::Binary { op, lhs, rhs, .. } => {
                self.emit(lhs)?;
                self.emit(rhs)?;
                self.push(Op::Binary(*op));
            }
            Expr::Call { name, args, span } => self.emit_call(name, args, span)?,
        }
        Ok(())
    }

    fn emit_call(&mut self, name: &str, args: &[Expr], span: &Span) -> Result<(), Diagnostic> {
        let Some(func) = FuncId::from_name(name) else {
            // Unknown function: `#NAME?`, and the arguments are not evaluated.
            let index = self.constant(Value::Error(ErrorKind::Name));
            self.push(Op::Const(index));
            return Ok(());
        };

        if !func.accepts(args.len()) {
            return Err(Diagnostic::new(
                span.clone(),
                format!(
                    "{} expects {}, but {} {} given",
                    func,
                    func.arity_rule(),
                    args.len(),
                    if args.len() == 1 { "was" } else { "were" }
                ),
            ));
        }

        if func == FuncId::If {
            return self.emit_lazy_if(args, span);
        }

        for arg in args {
            self.emit(arg)?;
        }
        self.push(Op::Call { func, argc: args.len() });
        Ok(())
    }

    /// `IF(cond, then[, else])` compiles to branches so that the untaken side is
    /// never evaluated.
    fn emit_lazy_if(&mut self, args: &[Expr], _span: &Span) -> Result<(), Diagnostic> {
        self.emit(&args[0])?;
        let jump_if_false = self.push(Op::JumpIfFalse(usize::MAX));
        self.emit(&args[1])?;
        let jump_over_else = self.push(Op::Jump(usize::MAX));

        let else_start = self.code.len();
        match args.get(2) {
            Some(alternative) => self.emit(alternative)?,
            // `IF(cond, then)` yields FALSE when the condition fails, as in Excel.
            None => {
                let index = self.constant(Value::Bool(false));
                self.push(Op::Const(index));
            }
        }

        let end = self.code.len();
        self.code[jump_if_false] = Op::JumpIfFalse(else_start);
        self.code[jump_over_else] = Op::Jump(end);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(src: &str) -> Program {
        compile_source(src).unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    #[test]
    fn compiles_arithmetic_to_postfix() {
        let p = program("=1+2*3");
        assert_eq!(
            p.code(),
            &[
                Op::Const(0),
                Op::Const(1),
                Op::Const(2),
                Op::Binary(BinOp::Mul),
                Op::Binary(BinOp::Add),
            ]
        );
        assert_eq!(p.consts(), &[Value::Number(1.0), Value::Number(2.0), Value::Number(3.0)]);
    }

    #[test]
    fn compiles_calls_with_their_arity() {
        let p = program("=SUM(1, 2, A1)");
        assert_eq!(p.code().last(), Some(&Op::Call { func: FuncId::Sum, argc: 3 }));
    }

    #[test]
    fn lazy_if_has_branches() {
        let p = program("=IF(A1, 1, 2)");
        assert!(p.code().iter().any(|op| matches!(op, Op::JumpIfFalse(_))));
        assert!(p.code().iter().any(|op| matches!(op, Op::Jump(_))));
        // Both branch targets must land inside the program.
        for op in p.code() {
            match op {
                Op::JumpIfFalse(target) | Op::Jump(target) => assert!(*target <= p.len()),
                _ => {}
            }
        }
    }

    #[test]
    fn if_with_two_arguments_defaults_the_else_branch_to_false() {
        let p = program("=IF(A1, 1)");
        assert!(p.consts().contains(&Value::Bool(false)));
    }

    #[test]
    fn unknown_functions_compile_to_name_errors() {
        let p = program("=NOPE(1, 2)");
        assert_eq!(p.code(), &[Op::Const(0)]);
        assert_eq!(p.consts(), &[Value::Error(ErrorKind::Name)]);
    }

    #[test]
    fn wrong_arity_is_a_compile_diagnostic_pointing_at_the_call() {
        let err = compile_source("=1+IF(A1)").unwrap_err();
        assert_eq!(err.message, "IF expects 2 or 3 arguments, but 1 was given");
        assert_eq!(err.span, (3, 9));
    }

    #[test]
    fn precedents_are_collected_once_each() {
        let p = program("=A1+A1+B2");
        assert_eq!(
            p.precedents(),
            vec![
                Precedent::Cell(crate::CellRef::new(0, 0)),
                Precedent::Cell(crate::CellRef::new(1, 1)),
            ]
        );
        let p = program("=SUM(A1:B2, A1)");
        assert_eq!(p.precedents().len(), 2);
    }

    #[test]
    fn off_sheet_references_are_not_precedents() {
        let p = program("=A1+#REF!");
        assert_eq!(p.precedents(), vec![Precedent::Cell(crate::CellRef::new(0, 0))]);
    }

    #[test]
    fn constants_are_deduplicated() {
        let p = program("=1+1+1");
        assert_eq!(p.consts().len(), 1);
    }
}
