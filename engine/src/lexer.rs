//! Formula lexer (built with `logos`).
//!
//! The lexer is deliberately *lexical*: it does not know the grammar. The one
//! judgement call it makes is that a token shaped like an `A1` reference is lexed
//! as a reference even though it also fits the shape of an identifier — that is
//! resolved by declaring the reference pattern first, which wins ties (both
//! patterns match `A1` for exactly two bytes). Undecidable cases (`A0`, a column
//! past `XFD`) are lexed as references here and resolved to `#NAME?` by the parser.
//!
//! Every token carries a byte span so parse errors can point at the exact
//! character that failed.

use std::fmt;

use logos::Logos;

use crate::addr::{Ref, MAX_COLS, MAX_ROWS};

/// A lexical token of the formula language.
#[derive(Logos, Clone, Debug, PartialEq)]
#[logos(skip r"[ \t\r\n]+")]
pub enum Token {
    // --- operators and punctuation -----------------------------------------
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

    // --- literals ----------------------------------------------------------
    /// `A1`, `$A$1`, `B$7` — validated later by the parser.
    #[regex(r"\$?[A-Za-z]{1,3}\$?[0-9]+", |lex| lex.slice().to_string())]
    CellRef(String),
    /// A bare name: function name (`SUM`) or a boolean literal (`TRUE`).
    #[regex(r"[A-Za-z][A-Za-z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),
    /// An error literal such as `#DIV/0!` or `#N/A`.
    ///
    /// The pattern is anchored rather than a bare `#[A-Za-z0-9/?!]+` character
    /// class: an error literal embeds `/` and digits, so a greedy class would
    /// swallow whatever follows it and `=#DIV/0!/2` would become a single token
    /// instead of `#DIV/0!`, `/`, `2`.
    #[regex(r"#(?:[A-Za-z0-9/]+!|N/A|NAME\?)", |lex| lex.slice().to_string(), ignore(case))]
    ErrorLiteral(String),
    /// A number literal. `inf`/`NaN` (reachable via `1e400`) is rejected here so
    /// that non-finite numbers can never enter the AST.
    #[regex(
        r"[0-9]+(\.[0-9]*)?([eE][+-]?[0-9]+)?|\.[0-9]+([eE][+-]?[0-9]+)?",
        |lex| lex.slice().parse::<f64>().ok().filter(|n| n.is_finite())
    )]
    Number(f64),
    /// A double-quoted string; `""` inside the string is a literal quote.
    #[regex(r#""([^"]|"")*""#, |lex| unescape_string(lex.slice()))]
    String(String),

    /// Input that matches no pattern (never produced directly by `logos`, which
    /// reports it as an error; the lexer surfaces it as this variant so that the
    /// parser can report it with position information).
    Unknown,
}

impl Token {
    /// A human readable name used in parse error messages.
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

    /// If this token is a cell reference that resolves to a real cell, the parsed
    /// reference; `A0` and `ZZZZ1`-style tokens return `None`.
    pub fn as_ref(&self) -> Option<Ref> {
        match self {
            Token::CellRef(s) => Ref::parse(s),
            _ => None,
        }
    }

    /// If this token is a bare name, its text.
    pub fn as_name(&self) -> Option<&str> {
        match self {
            Token::Ident(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// A token together with the byte range it came from.
pub type SpannedToken = (Token, std::ops::Range<usize>);

/// Tokenize `src`. Unrecognized characters become [`Token::Unknown`] so that the
/// parser can report them with a position instead of failing the whole lex.
pub fn lex(src: &str) -> Vec<SpannedToken> {
    Token::lexer(src)
        .spanned()
        .map(|(tok, span)| (tok.unwrap_or(Token::Unknown), span))
        .collect()
}

/// Whether `s` is a syntactically valid `A1`-style reference within sheet bounds.
pub fn is_valid_ref(s: &str) -> bool {
    Ref::parse(s).is_some()
}

/// Convert a `logos`-style string literal (including the quotes) to its value.
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

/// Bounds of the sheet, re-exported for lexer-adjacent diagnostics.
pub const SHEET_BOUNDS: (u32, u32) = (MAX_ROWS, MAX_COLS);

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
        // `A1B2` is an identifier (4 bytes) not a reference (2 bytes).
        assert_eq!(tokens("A1B2"), vec![Token::Ident("A1B2".into())]);
        // `<>` is not `<` followed by `>`.
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
        // Non-finite literals are rejected rather than entering the AST.
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
        // An unterminated string is an unknown token followed by an identifier.
        let lexed = lex(r#""abc"#);
        assert_eq!(lexed[0].0, Token::Unknown);
    }

    #[test]
    fn error_literals_are_lexed() {
        assert_eq!(tokens("#DIV/0!"), vec![Token::ErrorLiteral("#DIV/0!".into())]);
        assert_eq!(tokens("#N/A"), vec![Token::ErrorLiteral("#N/A".into())]);
        assert_eq!(tokens("#NAME?"), vec![Token::ErrorLiteral("#NAME?".into())]);
        // Case is preserved in the token; lookup is case-insensitive.
        assert_eq!(tokens("#div/0!"), vec![Token::ErrorLiteral("#div/0!".into())]);
        assert_eq!(crate::error::ErrorKind::from_literal("#div/0!"), Some(crate::error::ErrorKind::Div0));
        // An unknown `#...!` still lexes as one token (it becomes `#NAME?`).
        assert_eq!(tokens("#FOO!"), vec![Token::ErrorLiteral("#FOO!".into())]);
    }

    #[test]
    fn error_literals_do_not_swallow_following_operators() {
        // Regression: `#DIV/0!/2` used to lex as a single error literal.
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
            tokens("#NAME?&"),  // `&` is unknown, but the literal stops first
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
