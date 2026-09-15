//! First-class spreadsheet error values and shared diagnostics.
//!
//! Errors in Lattice are *values*, not control flow: any operation may produce an
//! error value, and error values propagate through the evaluator (an error operand
//! yields that error as the result of the operation). This mirrors Excel/Sheets
//! semantics and keeps a bad cell from taking down a recalculation.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The set of spreadsheet error values understood by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ErrorKind {
    /// `#DIV/0!` — division by zero.
    Div0,
    /// `#REF!` — a reference that points outside the sheet (e.g. produced by a fill).
    Ref,
    /// `#VALUE!` — an operand of the wrong type.
    Value,
    /// `#NAME?` — an unknown function name.
    Name,
    /// `#NUM!` — a numerically invalid operation (overflow, non-finite result, bad domain).
    Num,
    /// `#N/A` — a value is not available.
    NA,
    /// `#CYCLE!` — the cell participates in a circular reference.
    Cycle,
    /// `#PARSE!` — the stored formula text could not be parsed.
    Parse,
}

impl ErrorKind {
    /// The literal text of the error as displayed in a cell.
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

    /// Parse an error literal such as `#DIV/0!` (case-insensitive).
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

/// A message tied to a byte range of the formula source text.
///
/// Used for both lexical and syntactic errors so the UI can point at the exact
/// character where a formula fails.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Byte offsets into the original formula source (`start..end`).
    pub span: (usize, usize),
    /// Human readable description, e.g. "unexpected `)`".
    pub message: String,
}

impl Diagnostic {
    pub fn new(span: std::ops::Range<usize>, message: impl Into<String>) -> Self {
        Diagnostic { span: (span.start, span.end), message: message.into() }
    }

    /// 0-based byte range of the offending text.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.span.0..self.span.1
    }

    /// The offending slice of `src`, if it is still available.
    pub fn highlight<'a>(&self, src: &'a str) -> &'a str {
        src.get(self.range()).unwrap_or("")
    }

    /// Render a caret-annotated snippet, e.g.
    ///
    /// ```text
    /// =1 + 2)
    ///        ^ unexpected `)`
    /// ```
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
