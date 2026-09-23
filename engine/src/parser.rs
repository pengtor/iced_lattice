use chumsky::error::{RichPattern, RichReason};
use chumsky::input::{Input as _, MappedInput, Stream};
use chumsky::prelude::*;
use chumsky::span::{SimpleSpan, Spanned};

use crate::addr::{RangeRef, Ref};
use crate::ast::{BinOp, Expr, Span, UnOp};
use crate::error::{Diagnostic, ErrorKind};
use crate::lexer::{lex, Token};

type Extra<'a> = extra::Err<Rich<'a, Token>>;

type TokenStream = std::vec::IntoIter<(Token, SimpleSpan)>;
type Tokens<'a> = MappedInput<'a, Token, SimpleSpan, Stream<TokenStream>>;

pub fn parse(src: &str) -> Result<Expr, Diagnostic> {
    match run(src) {
        Ok(expr) => Ok(expr),
        Err(errs) => match errs.into_iter().next() {
            Some(diag) => Err(diag),
            None => Err(Diagnostic::new(0..src.len(), "invalid formula")),
        },
    }
}

pub fn parse_all_errors(src: &str) -> Result<Expr, Vec<Diagnostic>> {
    run(src)
}

fn run(src: &str) -> Result<Expr, Vec<Diagnostic>> {
    let tokens: Vec<(Token, SimpleSpan)> = lex(src).into_iter().map(|(t, s)| (t, s.into())).collect();
    let eoi: SimpleSpan = (src.len()..src.len()).into();
    // fn pointer (not closure) keeps the input type nameable.
    let split: fn((Token, SimpleSpan)) -> (Token, SimpleSpan) = |(token, span)| (token, span);
    let input = Stream::from_iter(tokens).map(eoi, split);
    let outcome = formula_parser().then_ignore(end()).parse(input).into_result();
    match outcome {
        Ok(expr) => Ok(expr),
        Err(errs) => Err(errs.iter().map(|e| to_diagnostic(e, src)).collect()),
    }
}

fn formula_parser<'a>() -> impl Parser<'a, Tokens<'a>, Expr, Extra<'a>> + Clone {
    recursive(|expr| {
        let number = select! { Token::Number(n) => n }
            .spanned()
            .map(|n: Spanned<f64>| Expr::Number(n.inner, span_of(&n)));

        let text = select! { Token::String(s) => s }
            .spanned()
            .map(|s: Spanned<String>| {
                let span = span_of(&s);
                Expr::Text(s.inner, span)
            });

        let error_literal = select! { Token::ErrorLiteral(s) => s }
            .spanned()
            .map(|s: Spanned<String>| {
                let span = span_of(&s);
                match ErrorKind::from_literal(&s.inner) {
                    Some(kind) => Expr::Error(kind, span),
                    None => Expr::Name(s.inner, span),
                }
            });

        // Out-of-bounds references become names, evaluating to #NAME?.
        let cell = select! { Token::CellRef(s) => s }.spanned();
        let reference = cell
            .then(just(Token::Colon).ignore_then(cell).or_not())
            .map(|(start, end): (Spanned<String>, Option<Spanned<String>>)| {
                let span = match &end {
                    Some(end) => start.span.start..end.span.end,
                    None => span_of(&start),
                };
                build_reference(start.inner, end.map(|e| e.inner), span)
            })
            .labelled("a reference");

        let ident = select! { Token::Ident(s) => s }.spanned();
        let arguments = expr.clone().separated_by(just(Token::Comma)).collect::<Vec<_>>();
        let call_or_name = ident
            .then(arguments.delimited_by(just(Token::LParen), just(Token::RParen)).or_not())
            .spanned()
            .map(|node: Spanned<(Spanned<String>, Option<Vec<Expr>>)>| {
                let node_span = span_of(&node);
                match node.inner {
                    (name, Some(args)) => Expr::Call { name: name.inner, args, span: node_span },
                    (name, None) => {
                        let name_span = span_of(&name);
                        boolean_or_name(name.inner, name_span)
                    }
                }
            });

        let atom = choice((number, text, error_literal, reference, call_or_name));

        let parenthesized = expr
            .clone()
            .delimited_by(just(Token::LParen), just(Token::RParen))
            .spanned()
            .map(|node: Spanned<Expr>| {
                let span = span_of(&node);
                node.inner.with_span(span)
            });

        let primary = choice((atom, parenthesized));

        let percent_op = select! { Token::Percent => () }.spanned();
        let postfix = primary.foldl(percent_op.repeated(), |operand, op: Spanned<()>| {
            let span = operand.span().start..op.span.end;
            Expr::Percent { operand: Box::new(operand), span }
        });

        let unary_op = choice((
            select! { Token::Minus => UnOp::Neg }.spanned(),
            select! { Token::Plus => UnOp::Plus }.spanned(),
        ));
        let unary = unary_op
            .repeated()
            .collect::<Vec<_>>()
            .then(postfix)
            .map(|(ops, operand): (Vec<Spanned<UnOp>>, Expr)| {
                ops.into_iter().rev().fold(operand, |operand, op| {
                    let span = op.span.start..operand.span().end;
                    Expr::Unary { op: op.inner, operand: Box::new(operand), span }
                })
            });

        // Unary binds tighter than ^: -2^2 is (-2)^2.
        let power = unary
            .clone()
            .foldl(just(Token::Caret).ignore_then(unary).repeated(), |lhs, rhs| {
                binary(BinOp::Pow, lhs, rhs)
            });

        let mul_op = choice((
            select! { Token::Star => BinOp::Mul },
            select! { Token::Slash => BinOp::Div },
        ));
        let multiplicative = power
            .clone()
            .foldl(mul_op.then(power).repeated(), |lhs, (op, rhs)| binary(op, lhs, rhs));

        let add_op = choice((
            select! { Token::Plus => BinOp::Add },
            select! { Token::Minus => BinOp::Sub },
        ));
        let additive = multiplicative
            .clone()
            .foldl(add_op.then(multiplicative).repeated(), |lhs, (op, rhs)| binary(op, lhs, rhs));

        let cmp_op = choice((
            select! { Token::Eq => BinOp::Eq },
            select! { Token::Ne => BinOp::Ne },
            select! { Token::Lt => BinOp::Lt },
            select! { Token::Le => BinOp::Le },
            select! { Token::Gt => BinOp::Gt },
            select! { Token::Ge => BinOp::Ge },
        ));
        let comparison = additive
            .clone()
            .foldl(cmp_op.then(additive).repeated(), |lhs, (op, rhs)| binary(op, lhs, rhs));

        just(Token::Eq).or_not().ignore_then(comparison)
    })
}

fn span_of<T>(spanned: &Spanned<T>) -> Span {
    spanned.span.into_range()
}

fn binary(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = lhs.span().start..rhs.span().end;
    Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs), span }
}

fn build_reference(name: String, end: Option<String>, span: Span) -> Expr {
    let start_ref = Ref::parse(&name);
    match end {
        None => match start_ref {
            Some(r) => Expr::Ref(r, span),
            None => Expr::Name(name, span),
        },
        Some(end_name) => match (start_ref, Ref::parse(&end_name)) {
            (Some(start), Some(end)) => Expr::Range(RangeRef { start, end }, span),
            // Unparseable range corner becomes #NAME?, not a range.
            _ => Expr::Error(ErrorKind::Name, span),
        },
    }
}

fn boolean_or_name(name: String, span: Span) -> Expr {
    match name.to_ascii_uppercase().as_str() {
        "TRUE" => Expr::Bool(true, span),
        "FALSE" => Expr::Bool(false, span),
        _ => Expr::Name(name, span),
    }
}

fn to_diagnostic(err: &Rich<'_, Token>, src: &str) -> Diagnostic {
    let mut span = err.span().into_range();
    let message = match err.reason() {
        RichReason::Custom(msg) => msg.clone(),
        RichReason::ExpectedFound { expected, .. } => {
            let found = match err.found() {
                Some(Token::Unknown) => describe_unknown(src, &span),
                Some(tok) => format!("unexpected {}", tok.describe()),
                None => "unexpected end of formula".to_string(),
            };
            // Lexical errors: omit expected list; narrow span to the character.
            if matches!(err.found(), Some(Token::Unknown)) {
                let width = src.get(span.start..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
                span = span.start..(span.start + width);
                found
            } else {
                let expected = describe_expected(expected);
                if expected.is_empty() {
                    found
                } else {
                    format!("{found}; expected {expected}")
                }
            }
        }
    };
    Diagnostic::new(span, message)
}

fn describe_unknown(src: &str, span: &Span) -> String {
    match src.get(span.start..).and_then(|s| s.chars().next()) {
        Some('"') => "unterminated string literal".to_string(),
        Some(c) => format!("unexpected character `{c}`"),
        None => "unexpected end of formula".to_string(),
    }
}

fn describe_expected(expected: &[RichPattern<'_, Token>]) -> String {
    const MAX: usize = 4;
    let mut names: Vec<String> = Vec::new();
    for pattern in expected {
        let name = match pattern {
            RichPattern::Token(t) => t.describe(),
            RichPattern::Label(l) => l.to_string(),
            RichPattern::Identifier(i) => format!("`{i}`"),
            RichPattern::EndOfInput => "end of formula".to_string(),
            _ => continue,
        };
        if name == "<unexpected>" || names.contains(&name) {
            continue;
        }
        names.push(name);
    }
    if names.is_empty() {
        return String::new();
    }
    let extra = names.len().saturating_sub(MAX);
    names.truncate(MAX);
    let joined = match names.len() {
        0 => String::new(),
        1 => names[0].clone(),
        n => format!("{} or {}", names[..n - 1].join(", "), names[n - 1]),
    };
    if extra > 0 {
        format!("{joined} (and {extra} more)")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Expr {
        parse(src).unwrap_or_else(|e| panic!("failed to parse {src:?}: {e}"))
    }

    fn parse_err(src: &str) -> Diagnostic {
        parse(src).err().unwrap_or_else(|| panic!("expected {src:?} to fail"))
    }

    #[test]
    fn parses_numbers_and_arithmetic() {
        assert_eq!(parse_ok("=1").to_string(), "1");
        assert_eq!(parse_ok("=1+2").to_string(), "1+2");
        assert_eq!(parse_ok("=1+2*3").to_string(), "1+2*3");
        assert_eq!(parse_ok("=(1+2)*3").to_string(), "(1+2)*3");
        assert_eq!(parse_ok("=1.5e2").to_string(), "150");
        assert_eq!(parse_ok("=.5").to_string(), "0.5");
        assert_eq!(parse_ok("=1 - 2").to_string(), "1-2");
    }

    #[test]
    fn operator_precedence_follows_the_grammar() {
        match parse_ok("=1+2*3") {
            Expr::Binary { op: BinOp::Add, rhs, .. } => {
                assert!(matches!(*rhs, Expr::Binary { op: BinOp::Mul, .. }))
            }
            other => panic!("unexpected shape: {other:?}"),
        }
        match parse_ok("=-2^2") {
            Expr::Binary { op: BinOp::Pow, lhs, .. } => {
                assert!(matches!(*lhs, Expr::Unary { op: UnOp::Neg, .. }))
            }
            other => panic!("unexpected shape: {other:?}"),
        }
        match parse_ok("=2^3^2") {
            Expr::Binary { op: BinOp::Pow, lhs, .. } => {
                assert!(matches!(*lhs, Expr::Binary { op: BinOp::Pow, .. }))
            }
            other => panic!("unexpected shape: {other:?}"),
        }
        match parse_ok("=1+2>2*1") {
            Expr::Binary { op: BinOp::Gt, lhs, .. } => {
                assert!(matches!(*lhs, Expr::Binary { op: BinOp::Add, .. }))
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn parses_references_ranges_and_anchors() {
        assert_eq!(parse_ok("=A1").to_string(), "A1");
        assert_eq!(parse_ok("=$a$1").to_string(), "$A$1");
        assert_eq!(parse_ok("=SUM(A1:B10)").to_string(), "SUM(A1:B10)");
        match parse_ok("=A1:B2") {
            Expr::Range(r, _) => {
                assert_eq!(r.start.to_cell().unwrap().a1(), "A1");
                assert_eq!(r.end.to_cell().unwrap().a1(), "B2");
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn parses_calls_strings_and_booleans() {
        assert_eq!(parse_ok("=SUM(1, 2, 3)").to_string(), "SUM(1, 2, 3)");
        assert_eq!(parse_ok("=SUM()").to_string(), "SUM()");
        assert_eq!(parse_ok("=IF(A1>1, \"yes\", \"no\")").to_string(), "IF(A1>1, \"yes\", \"no\")");
        assert_eq!(parse_ok("=TRUE").to_string(), "TRUE");
        assert_eq!(parse_ok("=false").to_string(), "FALSE");
        assert_eq!(parse_ok("=TOTAL").to_string(), "TOTAL");
        assert_eq!(parse_ok("=#DIV/0!").to_string(), "#DIV/0!");
    }

    #[test]
    fn parses_percent_postfix() {
        assert_eq!(parse_ok("=50%").to_string(), "50%");
        assert_eq!(parse_ok("=1+50%").to_string(), "1+50%");
        match parse_ok("=(1+2)%") {
            Expr::Percent { operand, .. } => assert!(matches!(*operand, Expr::Binary { .. })),
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn function_call_argument_lists_are_parsed() {
        match parse_ok("=CONCAT(\"a\", \"b\", A1)") {
            Expr::Call { name, args, .. } => {
                assert_eq!(name, "CONCAT");
                assert_eq!(args.len(), 3);
            }
            other => panic!("unexpected shape: {other:?}"),
        }
    }

    #[test]
    fn errors_point_at_the_offending_character() {
        let d = parse_err("=1 + 2)");
        assert_eq!(d.span, (6, 7));
        assert!(d.message.contains("unexpected"), "message was {:?}", d.message);
        assert!(
            d.render("=1 + 2)").starts_with("=1 + 2)\n      ^"),
            "caret misplaced: {:?}",
            d.render("=1 + 2)")
        );
    }

    #[test]
    fn incomplete_input_reports_the_end_of_the_formula() {
        let d = parse_err("=1+");
        assert_eq!(d.span, (3, 3));
        assert!(d.message.contains("unexpected end of formula"), "message was {:?}", d.message);
    }

    #[test]
    fn unknown_characters_are_reported_positionally() {
        let d = parse_err("=1 @ 2");
        assert_eq!(d.span, (3, 4));
        assert_eq!(d.message, "unexpected character `@`");
        let d = parse_err("=\"abc");
        assert_eq!(d.message, "unterminated string literal");
        assert_eq!(d.span, (1, 2));
    }

    #[test]
    fn bad_reference_shapes_become_names_not_parse_errors() {
        assert_eq!(parse_ok("=A0").to_string(), "A0");
        assert_eq!(parse_ok("=A0:B2").to_string(), "#NAME?");
    }

    #[test]
    fn mismatched_parenthesis_is_reported_inside_the_argument_list() {
        let d = parse_err("=SUM(1, 2");
        assert!(!d.message.is_empty());
        assert!(d.span.0 >= 3, "span should be inside the argument list: {d:?}");
    }

    #[test]
    fn empty_input_is_an_error() {
        let d = parse_err("");
        assert!(d.message.contains("unexpected end of formula"));
        let d = parse_err("=");
        assert!(d.message.contains("unexpected end of formula"));
    }

    #[test]
    fn every_error_has_a_caret_renderable_span() {
        for bad in ["=1+", "=)", "=(1", "=1 2", "=SUM(,)", "=\"a", "=@", "=SUM(1,"] {
            let d = parse_err(bad);
            assert!(d.span.0 <= bad.len() && d.span.1 <= bad.len(), "{bad}: bad span {d:?}");
            assert!(!d.message.is_empty(), "{bad}: empty message");
            let _ = d.render(bad);
        }
    }

    #[test]
    fn parse_all_errors_collects_diagnostics() {
        let errs = parse_all_errors("=1 2").unwrap_err();
        assert!(!errs.is_empty());
    }
}
