use std::fmt;

use crate::addr::{RangeRef, Ref};
use crate::error::ErrorKind;
use crate::value::format_number_exact;

pub type Span = std::ops::Range<usize>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

impl BinOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            BinOp::Eq => "=",
            BinOp::Ne => "<>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Pow => "^",
        }
    }

    // `^` binds looser than unary minus: `-2^2` is 4
    pub const fn precedence(self) -> u8 {
        match self {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 1,
            BinOp::Add | BinOp::Sub => 2,
            BinOp::Mul | BinOp::Div => 3,
            BinOp::Pow => 5,
        }
    }

    pub const fn is_comparison(self) -> bool {
        self.precedence() == 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Plus,
}

impl UnOp {
    pub const fn symbol(self) -> &'static str {
        match self {
            UnOp::Neg => "-",
            UnOp::Plus => "+",
        }
    }
}

const UNARY_PRECEDENCE: u8 = 6;
const PERCENT_PRECEDENCE: u8 = 7;
const ATOM_PRECEDENCE: u8 = 9;

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Number(f64, Span),
    Text(String, Span),
    Bool(bool, Span),
    Error(ErrorKind, Span),
    Ref(Ref, Span),
    Range(RangeRef, Span),
    // Unknown bare name evaluates to #NAME?
    Name(String, Span),
    Unary { op: UnOp, operand: Box<Expr>, span: Span },
    Percent { operand: Box<Expr>, span: Span },
    Binary { op: BinOp, lhs: Box<Expr>, rhs: Box<Expr>, span: Span },
    Call { name: String, args: Vec<Expr>, span: Span },
}

impl Expr {
    pub fn span(&self) -> &Span {
        match self {
            Expr::Number(_, s)
            | Expr::Text(_, s)
            | Expr::Bool(_, s)
            | Expr::Error(_, s)
            | Expr::Ref(_, s)
            | Expr::Range(_, s)
            | Expr::Name(_, s) => s,
            Expr::Unary { span, .. }
            | Expr::Percent { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Call { span, .. } => span,
        }
    }

    pub fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Unary { operand, .. } | Expr::Percent { operand, .. } => vec![operand],
            Expr::Binary { lhs, rhs, .. } => vec![lhs, rhs],
            Expr::Call { args, .. } => args.iter().collect(),
            _ => Vec::new(),
        }
    }

    fn precedence(&self) -> u8 {
        match self {
            Expr::Binary { op, .. } => op.precedence(),
            Expr::Unary { .. } => UNARY_PRECEDENCE,
            Expr::Percent { .. } => PERCENT_PRECEDENCE,
            _ => ATOM_PRECEDENCE,
        }
    }

    pub(crate) fn with_span(self, span: Span) -> Expr {
        match self {
            Expr::Number(v, _) => Expr::Number(v, span),
            Expr::Text(v, _) => Expr::Text(v, span),
            Expr::Bool(v, _) => Expr::Bool(v, span),
            Expr::Error(v, _) => Expr::Error(v, span),
            Expr::Ref(v, _) => Expr::Ref(v, span),
            Expr::Range(v, _) => Expr::Range(v, span),
            Expr::Name(v, _) => Expr::Name(v, span),
            Expr::Unary { op, operand, .. } => Expr::Unary { op, operand, span },
            Expr::Percent { operand, .. } => Expr::Percent { operand, span },
            Expr::Binary { op, lhs, rhs, .. } => Expr::Binary { op, lhs, rhs, span },
            Expr::Call { name, args, .. } => Expr::Call { name, args, span },
        }
    }

    // References leaving the sheet become #REF! error nodes
    pub fn shifted(&self, row_delta: i64, col_delta: i64) -> Expr {
        self.shifted_with(row_delta, col_delta, &mut |_| true)
    }

    fn shifted_with(&self, row_delta: i64, col_delta: i64, keep: &mut dyn FnMut(&Ref) -> bool) -> Expr {
        match self {
            Expr::Ref(r, span) => {
                if !keep(r) {
                    return self.clone();
                }
                match r.shifted(row_delta, col_delta).to_cell() {
                    Some(_) => Expr::Ref(r.shifted(row_delta, col_delta), span.clone()),
                    None => Expr::Error(ErrorKind::Ref, span.clone()),
                }
            }
            Expr::Range(range, span) => {
                let shifted = range.shifted(row_delta, col_delta);
                if shifted.is_valid() {
                    Expr::Range(shifted, span.clone())
                } else {
                    Expr::Error(ErrorKind::Ref, span.clone())
                }
            }
            Expr::Unary { op, operand, span } => Expr::Unary {
                op: *op,
                operand: Box::new(operand.shifted_with(row_delta, col_delta, keep)),
                span: span.clone(),
            },
            Expr::Percent { operand, span } => Expr::Percent {
                operand: Box::new(operand.shifted_with(row_delta, col_delta, keep)),
                span: span.clone(),
            },
            Expr::Binary { op, lhs, rhs, span } => Expr::Binary {
                op: *op,
                lhs: Box::new(lhs.shifted_with(row_delta, col_delta, keep)),
                rhs: Box::new(rhs.shifted_with(row_delta, col_delta, keep)),
                span: span.clone(),
            },
            Expr::Call { name, args, span } => Expr::Call {
                name: name.clone(),
                args: args.iter().map(|a| a.shifted_with(row_delta, col_delta, keep)).collect(),
                span: span.clone(),
            },
            _ => self.clone(),
        }
    }

    pub fn same_shape(&self, other: &Expr) -> bool {
        match (self, other) {
            (Expr::Number(a, _), Expr::Number(b, _)) => a == b,
            (Expr::Text(a, _), Expr::Text(b, _)) => a == b,
            (Expr::Bool(a, _), Expr::Bool(b, _)) => a == b,
            (Expr::Error(a, _), Expr::Error(b, _)) => a == b,
            (Expr::Ref(a, _), Expr::Ref(b, _)) => a == b,
            (Expr::Range(a, _), Expr::Range(b, _)) => a == b,
            (Expr::Name(a, _), Expr::Name(b, _)) => a == b,
            (Expr::Unary { op: a, operand: x, .. }, Expr::Unary { op: b, operand: y, .. }) => {
                a == b && x.same_shape(y)
            }
            (Expr::Percent { operand: x, .. }, Expr::Percent { operand: y, .. }) => x.same_shape(y),
            (
                Expr::Binary { op: a, lhs: l1, rhs: r1, .. },
                Expr::Binary { op: b, lhs: l2, rhs: r2, .. },
            ) => a == b && l1.same_shape(l2) && r1.same_shape(r2),
            (
                Expr::Call { name: a, args: x, .. },
                Expr::Call { name: b, args: y, .. },
            ) => a.eq_ignore_ascii_case(b) && x.len() == y.len() && x.iter().zip(y).all(|(p, q)| p.same_shape(q)),
            _ => false,
        }
    }

    pub fn size(&self) -> usize {
        1 + self.children().iter().map(|c| c.size()).sum::<usize>()
    }
}

fn print_with(
    f: &mut fmt::Formatter<'_>,
    expr: &Expr,
    parent_precedence: u8,
    right_operand: bool,
) -> fmt::Result {
    let own = expr.precedence();
    // left-assoc: same-precedence operand needs parens only on the right
    let needs_parens = own < parent_precedence || (own == parent_precedence && right_operand);
    if needs_parens {
        f.write_str("(")?;
    }
    match expr {
        Expr::Number(v, _) => f.write_str(&format_number_exact(*v))?,
        Expr::Text(s, _) => write!(f, "\"{}\"", s.replace('"', "\"\""))?,
        Expr::Bool(b, _) => f.write_str(if *b { "TRUE" } else { "FALSE" })?,
        Expr::Error(k, _) => write!(f, "{k}")?,
        Expr::Ref(r, _) => write!(f, "{r}")?,
        Expr::Range(r, _) => write!(f, "{r}")?,
        Expr::Name(n, _) => f.write_str(n)?,
        Expr::Unary { op, operand, .. } => {
            f.write_str(op.symbol())?;
            // `- -1` would parse as two ops; keep operand tight
            print_with(f, operand, UNARY_PRECEDENCE, false)?;
        }
        Expr::Percent { operand, .. } => {
            print_with(f, operand, PERCENT_PRECEDENCE, false)?;
            f.write_str("%")?;
        }
        Expr::Binary { op, lhs, rhs, .. } => {
            print_with(f, lhs, op.precedence(), false)?;
            f.write_str(op.symbol())?;
            print_with(f, rhs, op.precedence(), true)?;
        }
        Expr::Call { name, args, .. } => {
            f.write_str(name)?;
            f.write_str("(")?;
            for (i, arg) in args.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                print_with(f, arg, 0, false)?;
            }
            f.write_str(")")?;
        }
    }
    if needs_parens {
        f.write_str(")")?;
    }
    Ok(())
}

impl fmt::Display for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        print_with(f, self, 0, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn num(v: f64) -> Box<Expr> {
        Box::new(Expr::Number(v, 0..0))
    }

    fn bin(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
        Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span: 0..0 }
    }

    #[test]
    fn printer_adds_parentheses_only_where_needed() {
        let add = bin(BinOp::Add, Expr::Number(1.0, 0..0), Expr::Number(2.0, 0..0));
        let mul = bin(BinOp::Mul, add.clone(), Expr::Number(3.0, 0..0));
        assert_eq!(mul.to_string(), "(1+2)*3");
        let add2 = bin(BinOp::Add, Expr::Number(3.0, 0..0), mul.clone());
        assert_eq!(add2.to_string(), "3+(1+2)*3");
    }

    #[test]
    fn printer_handles_left_associative_right_operand() {
        let inner = bin(BinOp::Sub, Expr::Number(2.0, 0..0), Expr::Number(3.0, 0..0));
        let outer = bin(BinOp::Sub, Expr::Number(1.0, 0..0), inner);
        assert_eq!(outer.to_string(), "1-(2-3)");
        let same_left = bin(BinOp::Sub, outer.clone(), Expr::Number(4.0, 0..0));
        assert_eq!(same_left.to_string(), "1-(2-3)-4");
    }

    #[test]
    fn printer_preserves_power_associativity() {
        let left = bin(
            BinOp::Pow,
            bin(BinOp::Pow, Expr::Number(2.0, 0..0), Expr::Number(3.0, 0..0)),
            Expr::Number(2.0, 0..0),
        );
        let right = bin(
            BinOp::Pow,
            Expr::Number(2.0, 0..0),
            bin(BinOp::Pow, Expr::Number(3.0, 0..0), Expr::Number(2.0, 0..0)),
        );
        assert_eq!(left.to_string(), "2^3^2");
        assert_eq!(right.to_string(), "2^(3^2)");
    }

    #[test]
    fn printer_parenthesises_unary_against_power() {
        let neg = Expr::Unary { op: UnOp::Neg, operand: num(2.0), span: 0..0 };
        let pow_of_neg = bin(BinOp::Pow, neg.clone(), Expr::Number(2.0, 0..0));
        let neg_of_pow = Expr::Unary {
            op: UnOp::Neg,
            operand: Box::new(bin(BinOp::Pow, Expr::Number(2.0, 0..0), Expr::Number(2.0, 0..0))),
            span: 0..0,
        };
        assert_eq!(pow_of_neg.to_string(), "-2^2");
        assert_eq!(neg_of_pow.to_string(), "-(2^2)");
    }

    #[test]
    fn printing_then_parsing_is_stable() {
        for src in [
            "=1+2*3",
            "=(1+2)*3",
            "=1-(2-3)",
            "=2^3^2",
            "=-2^2",
            "=SUM(A1:B2, 3)",
            "=IF(A1>2, \"hi\"\", there\", B$2)",
            "=50%+1",
            "=-(2^2)",
            "=#REF!+1",
        ] {
            let first = parse(src).unwrap_or_else(|e| panic!("{src}: {e}"));
            let printed = first.to_string();
            let second = parse(&printed).unwrap_or_else(|e| panic!("{printed}: {e}"));
            assert!(
                first.same_shape(&second),
                "round trip changed the tree for {src} -> {printed}\n  {first:?}\n  {second:?}"
            );
        }
    }

    #[test]
    fn shifted_moves_relative_references_only() {
        let expr = parse("=A1+$B$2+C$3+$D4").unwrap();
        assert_eq!(expr.shifted(1, 1).to_string(), "B2+$B$2+D$3+$D5");
    }

    #[test]
    fn shifted_ranges_and_errors() {
        let expr = parse("=SUM(A1:B2)").unwrap();
        assert_eq!(expr.shifted(1, 0).to_string(), "SUM(A2:B3)");
        let expr = parse("=A1+1").unwrap();
        assert_eq!(expr.shifted(-1, 0).to_string(), "#REF!+1");
        let expr = parse("=SUM(A1:B2)").unwrap();
        assert_eq!(expr.shifted(-1, 0).to_string(), "SUM(#REF!)");
    }

    #[test]
    fn lex_and_parse_agree_on_spans() {
        let src = "=1+22";
        let expr = parse(src).unwrap();
        assert_eq!(expr.span(), &(1..5));
        let lexed = lex(src);
        assert_eq!(lexed.len(), 4);
    }

    #[test]
    fn size_counts_nodes() {
        assert_eq!(parse("=1+2").unwrap().size(), 3);
        assert_eq!(parse("=SUM(1,2,3)").unwrap().size(), 4);
    }
}
