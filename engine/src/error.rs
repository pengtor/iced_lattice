use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ErrorKind {
    Div0,
    Ref,
    Value,
    Name,
    Num,
    NA,
    Cycle,
    Parse,
}

impl ErrorKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            ErrorKind::Div0 => "#DIV/0!",
            ErrorKind::Ref => "#REF!",
            ErrorKind::Value => "#VALUE!",
            ErrorKind::Name => "#NAME?",
            ErrorKind::Num => "#NUM!",
            ErrorKind::NA => "#N/A",
            ErrorKind::Cycle => "#CYCLE!",
            ErrorKind::Parse => "#PARSE!",
        }
    }

    // Case-insensitive; surrounding whitespace ignored
    pub fn from_literal(s: &str) -> Option<Self> {
        const ALL: [ErrorKind; 8] = [
            ErrorKind::Div0,
            ErrorKind::Ref,
            ErrorKind::Value,
            ErrorKind::Name,
            ErrorKind::Num,
            ErrorKind::NA,
            ErrorKind::Cycle,
            ErrorKind::Parse,
        ];
        let upper = s.trim().to_ascii_uppercase();
        ALL.into_iter().find(|k| k.as_str() == upper)
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ErrorKind {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ErrorKind::from_literal(s).ok_or(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub span: (usize, usize),
    pub message: String,
}

impl Diagnostic {
    pub fn new(span: std::ops::Range<usize>, message: impl Into<String>) -> Self {
        Diagnostic { span: (span.start, span.end), message: message.into() }
    }

    pub fn range(&self) -> std::ops::Range<usize> {
        self.span.0..self.span.1
    }

    pub fn render(&self, src: &str) -> String {
        let range = self.range();
        let start = range.start.min(src.len());
        let end = range.end.min(src.len()).max(start);
        let mut out = String::with_capacity(src.len() + 8);
        out.push_str(src);
        out.push('\n');
        for _ in 0..start {
            out.push(' ');
        }
        for _ in start..end.max(start + 1) {
            out.push('^');
        }
        out.push(' ');
        out.push_str(&self.message);
        out
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at byte {}", self.message, self.span.0)
    }
}

impl std::error::Error for Diagnostic {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_literals_round_trip() {
        for k in [
            ErrorKind::Div0,
            ErrorKind::Ref,
            ErrorKind::Value,
            ErrorKind::Name,
            ErrorKind::Num,
            ErrorKind::NA,
            ErrorKind::Cycle,
            ErrorKind::Parse,
        ] {
            assert_eq!(ErrorKind::from_literal(k.as_str()), Some(k));
        }
    }

    #[test]
    fn error_literal_lookup_is_case_insensitive() {
        assert_eq!(ErrorKind::from_literal("#div/0!"), Some(ErrorKind::Div0));
        assert_eq!(ErrorKind::from_literal(" #REF! "), Some(ErrorKind::Ref));
        assert_eq!(ErrorKind::from_literal("#NOPE!"), None);
    }

    #[test]
    fn diagnostic_renders_caret_under_the_error() {
        let d = Diagnostic::new(6..7, "unexpected `)`");
        assert_eq!(d.render("=1 + 2)"), "=1 + 2)\n      ^ unexpected `)`");
    }

    #[test]
    fn diagnostic_renders_one_caret_for_an_empty_span() {
        let d = Diagnostic::new(2..2, "unexpected end of formula");
        assert_eq!(d.render("=1"), "=1\n  ^ unexpected end of formula");
    }
}
