use crate::ast::{BinOp, Expr, Span, UnOp};
use crate::error::{Diagnostic, ErrorKind};
use crate::value::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arity {
    Any,
    Exact(usize),
    Between(usize, usize),
    AtLeast(usize),
    Even(usize),
    Odd(usize),
}

impl Arity {
    pub const fn accepts(self, argc: usize) -> bool {
        match self {
            Arity::Any => true,
            Arity::Exact(n) => argc == n,
            Arity::Between(min, max) => argc >= min && argc <= max,
            Arity::AtLeast(min) => argc >= min,
            Arity::Even(min) => argc >= min && argc % 2 == 0,
            Arity::Odd(min) => argc >= min && argc % 2 == 1,
        }
    }

    pub const fn sample(self) -> usize {
        match self {
            Arity::Any => 0,
            Arity::Exact(n) => n,
            Arity::Between(min, _) | Arity::AtLeast(min) | Arity::Even(min) | Arity::Odd(min) => min,
        }
    }

    pub fn describe(self) -> String {
        fn plural(n: usize) -> &'static str {
            if n == 1 {
                "argument"
            } else {
                "arguments"
            }
        }
        match self {
            Arity::Any => "any number of arguments".to_string(),
            Arity::Exact(0) => "no arguments".to_string(),
            Arity::Exact(n) => format!("exactly {n} {}", plural(n)),
            Arity::Between(min, max) if max == min + 1 => format!("{min} or {max} arguments"),
            Arity::Between(min, max) => format!("{min} to {max} arguments"),
            Arity::AtLeast(min) => format!("at least {min} {}", plural(min)),
            Arity::Even(min) => format!("an even number of arguments (at least {min})"),
            Arity::Odd(min) => format!("an odd number of arguments (at least {min})"),
        }
    }
}

// IF/IFS/IFERROR/IFNA compile to branches, never dispatched
macro_rules! functions {
    ($($variant:ident, $name:literal, $arity:expr;)*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum FuncId {
            $($variant,)*
        }

        impl FuncId {
            pub const ALL: &'static [FuncId] = &[$(FuncId::$variant,)*];

            pub fn from_name(name: &str) -> Option<FuncId> {
                Some(match name.to_ascii_uppercase().as_str() {
                    $($name => FuncId::$variant,)*
                    _ => return None,
                })
            }

            pub const fn name(self) -> &'static str {
                match self {
                    $(FuncId::$variant => $name,)*
                }
            }

            pub const fn arity(self) -> Arity {
                match self {
                    $(FuncId::$variant => $arity,)*
                }
            }

            pub fn arity_rule(self) -> String {
                self.arity().describe()
            }

            pub fn accepts(self, argc: usize) -> bool {
                self.arity().accepts(argc)
            }
        }
    };
}

functions! {
    Sum, "SUM", Arity::Any;
    Average, "AVERAGE", Arity::Any;
    Count, "COUNT", Arity::Any;
    Min, "MIN", Arity::Any;
    Max, "MAX", Arity::Any;
    Concat, "CONCAT", Arity::Any;

    If, "IF", Arity::Between(2, 3);
    Ifs, "IFS", Arity::Even(2);
    IfError, "IFERROR", Arity::Exact(2);
    IfNa, "IFNA", Arity::Exact(2);

    And, "AND", Arity::AtLeast(1);
    Or, "OR", Arity::AtLeast(1);
    Not, "NOT", Arity::Exact(1);
    Xor, "XOR", Arity::AtLeast(1);

    IsBlank, "ISBLANK", Arity::Exact(1);
    IsNumber, "ISNUMBER", Arity::Exact(1);
    IsText, "ISTEXT", Arity::Exact(1);
    IsError, "ISERROR", Arity::Exact(1);
    IsNa, "ISNA", Arity::Exact(1);

    Round, "ROUND", Arity::Between(1, 2);
    RoundUp, "ROUNDUP", Arity::Between(1, 2);
    RoundDown, "ROUNDDOWN", Arity::Between(1, 2);
    Abs, "ABS", Arity::Exact(1);
    Sqrt, "SQRT", Arity::Exact(1);
    Power, "POWER", Arity::Exact(2);
    Mod, "MOD", Arity::Exact(2);
    Int, "INT", Arity::Exact(1);
    Trunc, "TRUNC", Arity::Between(1, 2);
    Ceiling, "CEILING", Arity::Between(1, 2);
    Floor, "FLOOR", Arity::Between(1, 2);
    Sign, "SIGN", Arity::Exact(1);

    Len, "LEN", Arity::Exact(1);
    Upper, "UPPER", Arity::Exact(1);
    Lower, "LOWER", Arity::Exact(1);
    Trim, "TRIM", Arity::Exact(1);
    Left, "LEFT", Arity::Between(1, 2);
    Right, "RIGHT", Arity::Between(1, 2);
    Mid, "MID", Arity::Exact(3);
    Find, "FIND", Arity::Between(2, 3);
    Search, "SEARCH", Arity::Between(2, 3);
    Substitute, "SUBSTITUTE", Arity::Between(3, 4);
    Replace, "REPLACE", Arity::Exact(4);
    Text, "TEXT", Arity::Exact(2);
    ValueFn, "VALUE", Arity::Exact(1);

    VLookup, "VLOOKUP", Arity::Between(3, 4);
    HLookup, "HLOOKUP", Arity::Between(3, 4);
    Index, "INDEX", Arity::Between(2, 3);
    Match, "MATCH", Arity::Between(2, 3);
    XLookup, "XLOOKUP", Arity::Between(3, 4);

    SumIf, "SUMIF", Arity::Between(2, 3);
    CountIf, "COUNTIF", Arity::Exact(2);
    AverageIf, "AVERAGEIF", Arity::Between(2, 3);
    SumIfs, "SUMIFS", Arity::Odd(3);
    CountIfs, "COUNTIFS", Arity::Even(2);
    AverageIfs, "AVERAGEIFS", Arity::Odd(3);

    // MEDIAN etc.: no arguments is #DIV/0! at eval, not parse
    Median, "MEDIAN", Arity::Any;
    Mode, "MODE", Arity::Any;
    StDev, "STDEV", Arity::Any;
    Var, "VAR", Arity::Any;

    Today, "TODAY", Arity::Exact(0);
    Now, "NOW", Arity::Exact(0);
    Date, "DATE", Arity::Exact(3);
    Year, "YEAR", Arity::Exact(1);
    Month, "MONTH", Arity::Exact(1);
    Day, "DAY", Arity::Exact(1);
    Weekday, "WEEKDAY", Arity::Between(1, 2);
    DateDif, "DATEDIF", Arity::Exact(3);
}

impl FuncId {
    // Volatile calls have no precedents; scheduler reseeds them each recalculation
    pub const fn is_volatile(self) -> bool {
        matches!(self, FuncId::Today | FuncId::Now)
    }
}

impl std::fmt::Display for FuncId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Const(u32),
    Ref(crate::addr::Ref),
    Range(crate::addr::RangeRef),
    Unary(UnOp),
    Percent,
    Binary(BinOp),
    Call { func: FuncId, argc: usize },
    JumpIfFalse(usize),
    Jump(usize),
    JumpIfError(usize),
    JumpIfNA(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    code: Vec<Op>,
    consts: Vec<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precedent {
    Cell(crate::addr::CellRef),
    // Ranges kept as rectangles, not expanded into per-cell edges
    Range(crate::addr::RangeRef),
}

impl Program {
    pub fn code(&self) -> &[Op] {
        &self.code
    }

    pub fn consts(&self) -> &[Value] {
        &self.consts
    }

    pub fn len(&self) -> usize {
        self.code.len()
    }

    pub fn is_empty(&self) -> bool {
        self.code.is_empty()
    }

    // Precedents feed the dependency graph; off-sheet refs are skipped
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

    pub fn is_volatile(&self) -> bool {
        self.code.iter().any(|op| matches!(op, Op::Call { func, .. } if func.is_volatile()))
    }
}

pub fn compile(expr: &Expr) -> Result<Program, Diagnostic> {
    let mut compiler = Compiler { code: Vec::new(), consts: Vec::new() };
    compiler.emit(expr)?;
    Ok(Program { code: compiler.code, consts: compiler.consts })
}

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

        match func {
            FuncId::If => return self.emit_lazy_if(args, span),
            FuncId::Ifs => return self.emit_lazy_ifs(args),
            FuncId::IfError => return self.emit_error_guard(args, false),
            FuncId::IfNa => return self.emit_error_guard(args, true),
            _ => {}
        }

        for arg in args {
            self.emit(arg)?;
        }
        self.push(Op::Call { func, argc: args.len() });
        Ok(())
    }

    fn emit_error_guard(&mut self, args: &[Expr], na_only: bool) -> Result<(), Diagnostic> {
        self.emit(&args[0])?;
        let catch = self.push(if na_only { Op::JumpIfNA(usize::MAX) } else { Op::JumpIfError(usize::MAX) });
        let keep = self.push(Op::Jump(usize::MAX));

        let fallback = self.code.len();
        self.emit(&args[1])?;

        let end = self.code.len();
        match &mut self.code[catch] {
            Op::JumpIfError(target) | Op::JumpIfNA(target) => *target = fallback,
            _ => unreachable!("just pushed a jump-if-error"),
        }
        self.code[keep] = Op::Jump(end);
        Ok(())
    }

    fn emit_lazy_ifs(&mut self, args: &[Expr]) -> Result<(), Diagnostic> {
        let mut jumps_to_end = Vec::new();
        for pair in args.chunks(2) {
            self.emit(&pair[0])?;
            let next_pair = self.push(Op::JumpIfFalse(usize::MAX));
            self.emit(&pair[1])?;
            jumps_to_end.push(self.push(Op::Jump(usize::MAX)));

            let here = self.code.len();
            self.code[next_pair] = Op::JumpIfFalse(here);
        }

        let index = self.constant(Value::Error(ErrorKind::NA));
        self.push(Op::Const(index));

        let end = self.code.len();
        for jump in jumps_to_end {
            self.code[jump] = Op::Jump(end);
        }
        Ok(())
    }

    fn emit_lazy_if(&mut self, args: &[Expr], _span: &Span) -> Result<(), Diagnostic> {
        self.emit(&args[0])?;
        let jump_if_false = self.push(Op::JumpIfFalse(usize::MAX));
        self.emit(&args[1])?;
        let jump_over_else = self.push(Op::Jump(usize::MAX));

        let else_start = self.code.len();
        match args.get(2) {
            Some(alternative) => self.emit(alternative)?,
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

    #[test]
    fn only_today_and_now_are_volatile() {
        let volatile: Vec<FuncId> =
            FuncId::ALL.iter().copied().filter(|func| func.is_volatile()).collect();
        assert_eq!(volatile, vec![FuncId::Today, FuncId::Now]);
    }

    #[test]
    fn a_program_is_volatile_when_any_call_is() {
        assert!(program("=TODAY()").is_volatile());
        assert!(program("=NOW()").is_volatile());
        assert!(program("=A1+TODAY()*2").is_volatile());
        assert!(program("=IF(A1, NOW(), 0)").is_volatile());
        assert!(program("=IFERROR(DATE(2024,1,1), TODAY())").is_volatile());
        assert!(!program("=A1+1").is_volatile());
        assert!(!program("=DATE(2024,1,1)").is_volatile());
        assert!(!program("=SUM(A1:A9)").is_volatile());
    }
}
