use std::fmt;

use logos::Logos;

use crate::addr::Ref;

#[derive(Logos, Clone, Debug, PartialEq)]
#[logos(skip r"[ \t\r\n]+")]
pub enum Token {
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("^")]
    Caret,
    #[token("%")]
    Percent,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token(",")]
    Comma,
    #[token(":")]
    Colon,
    #[token("=")]
    Eq,
    #[token("<>")]
    Ne,
    #[token("<=")]
    Le,
    #[token(">=")]
    Ge,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,

    // Ref shape only; parser bounds-checks (A0 becomes #NAME?)
    #[regex(r"\$?[A-Za-z]{1,3}\$?[0-9]+", |lex| lex.slice().to_string())]
    CellRef(String),
    #[regex(r"[A-Za-z][A-Za-z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),
    // Anchored so `#DIV/0!/2` doesn't lex as one token
    #[regex(r"#(?:[A-Za-z0-9/]+!|N/A|NAME\?)", |lex| lex.slice().to_string(), ignore(case))]
    ErrorLiteral(String),
    // Non-finite literals rejected so the AST never holds inf/NaN
    #[regex(
        r"[0-9]+(\.[0-9]*)?([eE][+-]?[0-9]+)?|\.[0-9]+([eE][+-]?[0-9]+)?",
        |lex| lex.slice().parse::<f64>().ok().filter(|n| n.is_finite())
    )]
    Number(f64),
    // Doubled "" inside a string is one literal quote
    #[regex(r#""([^"]|"")*""#, |lex| unescape_string(lex.slice()))]
    String(String),

    // Surfaces lex errors as a token for positioned parse errors
    Unknown,
}

impl Token {
    pub fn describe(&self) -> String {
        match self {
            Token::Plus => "`+`".into(),
            Token::Minus => "`-`".into(),
            Token::Star => "`*`".into(),
            Token::Slash => "`/`".into(),
            Token::Caret => "`^`".into(),
            Token::Percent => "`%`".into(),
            Token::LParen => "`(`".into(),
            Token::RParen => "`)`".into(),
            Token::Comma => "`,`".into(),
            Token::Colon => "`:`".into(),
            Token::Eq => "`=`".into(),
            Token::Ne => "`<>`".into(),
            Token::Le => "`<=`".into(),
            Token::Ge => "`>=`".into(),
            Token::Lt => "`<`".into(),
            Token::Gt => "`>`".into(),
            Token::CellRef(s) => format!("reference `{s}`"),
            Token::Ident(s) => format!("name `{s}`"),
            Token::ErrorLiteral(s) => format!("error literal `{s}`"),
            Token::Number(n) => format!("number `{}`", crate::value::format_number(*n)),
            Token::String(s) => format!("string {s:?}"),
            Token::Unknown => "<unexpected>".into(),
        }
    }

    pub fn as_ref(&self) -> Option<Ref> {
        match self {
            Token::CellRef(s) => Ref::parse(s),
            _ => None,
        }
    }

}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

pub type SpannedToken = (Token, std::ops::Range<usize>);

pub fn lex(src: &str) -> Vec<SpannedToken> {
    Token::lexer(src)
        .spanned()
        .map(|(tok, span)| (tok.unwrap_or(Token::Unknown), span))
        .collect()
}

pub fn is_valid_ref(s: &str) -> bool {
    Ref::parse(s).is_some()
}

fn unescape_string(literal: &str) -> String {
    let inner = literal.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(literal);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' && chars.peek() == Some(&'"') {
            chars.next();
            out.push('"');
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(src: &str) -> Vec<Token> {
        lex(src).into_iter().map(|(t, _)| t).collect()
    }

    #[test]
    fn lexes_arithmetic() {
        assert_eq!(
            tokens("1+2*3-(4/5)"),
            vec![
                Token::Number(1.0),
                Token::Plus,
                Token::Number(2.0),
                Token::Star,
                Token::Number(3.0),
                Token::Minus,
                Token::LParen,
                Token::Number(4.0),
                Token::Slash,
                Token::Number(5.0),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn lexes_references_with_anchors() {
        assert_eq!(
            tokens("$A$1 + B$7 + $C10"),
            vec![
                Token::CellRef("$A$1".into()),
                Token::Plus,
                Token::CellRef("B$7".into()),
                Token::Plus,
                Token::CellRef("$C10".into()),
            ]
        );
    }

    #[test]
    fn lexes_ranges_and_calls() {
        assert_eq!(
            tokens("SUM(A1:B10, 2)"),
            vec![
                Token::Ident("SUM".into()),
                Token::LParen,
                Token::CellRef("A1".into()),
                Token::Colon,
                Token::CellRef("B10".into()),
                Token::Comma,
                Token::Number(2.0),
                Token::RParen,
            ]
        );
    }

    #[test]
    fn longest_match_wins() {
        assert_eq!(tokens("A1B2"), vec![Token::Ident("A1B2".into())]);
        assert_eq!(tokens("<>"), vec![Token::Ne]);
        assert_eq!(tokens("<=>="), vec![Token::Le, Token::Ge]);
    }

    #[test]
    fn spans_are_exact_byte_ranges() {
        let lexed = lex("=A1 + 22");
        assert_eq!(lexed[0], (Token::Eq, 0..1));
        assert_eq!(lexed[1], (Token::CellRef("A1".into()), 1..3));
        assert_eq!(lexed[2], (Token::Plus, 4..5));
        assert_eq!(lexed[3], (Token::Number(22.0), 6..8));
    }

    #[test]
    fn lexes_numbers_including_exponents() {
        assert_eq!(tokens("1.5"), vec![Token::Number(1.5)]);
        assert_eq!(tokens(".5"), vec![Token::Number(0.5)]);
        assert_eq!(tokens("2e3"), vec![Token::Number(2000.0)]);
        assert_eq!(tokens("1E-2"), vec![Token::Number(0.01)]);
        assert_eq!(tokens("1e400"), vec![Token::Unknown]);
    }

    #[test]
    fn lexes_strings_with_escaped_quotes() {
        assert_eq!(tokens(r#""hi""#), vec![Token::String("hi".into())]);
        assert_eq!(tokens(r#""say ""hi"" now""#), vec![Token::String(r#"say "hi" now"#.into())]);
        assert_eq!(tokens(r#""""#), vec![Token::String(String::new())]);
    }

    #[test]
    fn unknown_input_becomes_a_token_not_a_panic() {
        let lexed = lex("1 @ 2");
        assert_eq!(lexed[1].0, Token::Unknown);
        assert_eq!(lexed[1].1, 2..3);
        let lexed = lex(r#""abc"#);
        assert_eq!(lexed[0].0, Token::Unknown);
    }

    #[test]
    fn error_literals_are_lexed() {
        assert_eq!(tokens("#DIV/0!"), vec![Token::ErrorLiteral("#DIV/0!".into())]);
        assert_eq!(tokens("#N/A"), vec![Token::ErrorLiteral("#N/A".into())]);
        assert_eq!(tokens("#NAME?"), vec![Token::ErrorLiteral("#NAME?".into())]);
        assert_eq!(tokens("#div/0!"), vec![Token::ErrorLiteral("#div/0!".into())]);
        assert_eq!(crate::error::ErrorKind::from_literal("#div/0!"), Some(crate::error::ErrorKind::Div0));
        assert_eq!(tokens("#FOO!"), vec![Token::ErrorLiteral("#FOO!".into())]);
    }

    #[test]
    fn error_literals_do_not_swallow_following_operators() {
        assert_eq!(
            tokens("#DIV/0!/2"),
            vec![Token::ErrorLiteral("#DIV/0!".into()), Token::Slash, Token::Number(2.0)]
        );
        assert_eq!(
            tokens("#N/A+1"),
            vec![Token::ErrorLiteral("#N/A".into()), Token::Plus, Token::Number(1.0)]
        );
        assert_eq!(
            tokens("#N/A/2"),
            vec![Token::ErrorLiteral("#N/A".into()), Token::Slash, Token::Number(2.0)]
        );
        assert_eq!(
            tokens("#NAME?&"),
            vec![Token::ErrorLiteral("#NAME?".into()), Token::Unknown]
        );
    }

    #[test]
    fn references_out_of_bounds_lex_as_refs_but_do_not_resolve() {
        assert_eq!(tokens("A0"), vec![Token::CellRef("A0".into())]);
        assert_eq!(Token::CellRef("A0".into()).as_ref(), None);
        assert!(!is_valid_ref("A0"));
        assert!(is_valid_ref("a1"));
        assert!(is_valid_ref("$XFD$1048576"));
    }
}
