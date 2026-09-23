use std::cmp::Ordering;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::ErrorKind;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    // unused cell; coerces to 0, "", or FALSE by context
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
    Error(ErrorKind),
}

impl Value {
    // callers must use finite_number for possibly non-finite results
    pub fn number(n: f64) -> Value {
        debug_assert!(n.is_finite(), "Value::number called with non-finite {n}");
        Value::Number(n)
    }

    pub fn finite_number(n: f64) -> Value {
        if n.is_finite() {
            Value::Number(n)
        } else {
            Value::Error(ErrorKind::Num)
        }
    }

    pub fn text(s: impl Into<String>) -> Value {
        Value::Text(s.into())
    }

    pub fn error(kind: ErrorKind) -> Value {
        Value::Error(kind)
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Value::Empty)
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }

    // Empty->0, Bool->1/0; text never parses, unlike Excel
    pub fn as_number(&self) -> Result<f64, ErrorKind> {
        match self {
            Value::Empty => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(_) => Err(ErrorKind::Value),
            Value::Error(k) => Err(*k),
        }
    }

    pub fn as_bool(&self) -> Result<bool, ErrorKind> {
        match self {
            Value::Empty => Ok(false),
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Text(_) => Err(ErrorKind::Value),
            Value::Error(k) => Err(*k),
        }
    }

    pub fn as_text(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Text(s) => s.clone(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Error(k) => k.to_string(),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Empty => "empty",
            Value::Number(_) => "number",
            Value::Text(_) => "text",
            Value::Bool(_) => "boolean",
            Value::Error(_) => "error",
        }
    }

    // cross-type: number < text < bool; text compare case-insensitive
    pub fn compare(&self, other: &Value) -> Result<Ordering, ErrorKind> {
        use Value::*;
        match (self, other) {
            (Error(k), _) => Err(*k),
            (_, Error(k)) => Err(*k),
            (Empty, Empty) => Ok(Ordering::Equal),
            (Empty, Number(n)) => number_cmp(&0.0, n),
            (Number(n), Empty) => number_cmp(n, &0.0),
            (Empty, Text(t)) => Ok(text_cmp("", t)),
            (Text(t), Empty) => Ok(text_cmp(t, "")),
            (Empty, Bool(b)) => Ok(false.cmp(b)),
            (Bool(b), Empty) => Ok(b.cmp(&false)),
            (Number(a), Number(b)) => number_cmp(a, b),
            (Text(a), Text(b)) => Ok(text_cmp(a, b)),
            (Bool(a), Bool(b)) => Ok(a.cmp(b)),
            (Number(_), Text(_)) | (Number(_), Bool(_)) => Ok(Ordering::Less),
            (Text(_), Bool(_)) => Ok(Ordering::Less),
            (Text(_), Number(_)) | (Bool(_), Number(_)) => Ok(Ordering::Greater),
            (Bool(_), Text(_)) => Ok(Ordering::Greater),
        }
    }
}

fn number_cmp(a: &f64, b: &f64) -> Result<Ordering, ErrorKind> {
    a.partial_cmp(b).ok_or(ErrorKind::Num)
}

fn text_cmp(a: &str, b: &str) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_text())
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(v)
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}

impl From<ErrorKind> for Value {
    fn from(k: ErrorKind) -> Self {
        Value::Error(k)
    }
}

pub fn format_number_exact(n: f64) -> String {
    format!("{n}")
}

const DISPLAY_SIGNIFICANT_DIGITS: i32 = 15;

pub fn format_number(n: f64) -> String {
    if !n.is_finite() {
        return ErrorKind::Num.to_string();
    }
    if n == 0.0 {
        return "0".to_string(); // also normalizes -0.0
    }
    let abs = n.abs();
    if !(1e-9..1e21).contains(&abs) {
        return trim_exponent(format!("{n:.14e}"));
    }
    let rounded = if abs < 1e15 { round_significant(n, DISPLAY_SIGNIFICANT_DIGITS) } else { n };
    format!("{rounded}")
}

fn round_significant(n: f64, digits: i32) -> f64 {
    if n == 0.0 {
        return 0.0;
    }
    let exponent = n.abs().log10().floor() as i32;
    let scale = 10f64.powi(digits - 1 - exponent);
    if !scale.is_finite() {
        return n;
    }
    let scaled = n * scale;
    if !scaled.is_finite() {
        return n;
    }
    scaled.round() / scale
}

fn trim_exponent(s: String) -> String {
    let (mantissa, exponent) = match s.split_once('e') {
        Some((m, e)) => (m, e),
        None => return s,
    };
    let mantissa = if let Some(stripped) = mantissa.strip_suffix('.') {
        stripped
    } else {
        mantissa.trim_end_matches('0').trim_end_matches('.')
    };
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(d) => ("-", d),
        None => ("", exponent.strip_prefix('+').unwrap_or(exponent)),
    };
    let digits = digits.trim_start_matches('0');
    format!("{mantissa}e{sign}{}", if digits.is_empty() { "0" } else { digits })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_coerces_per_context() {
        assert_eq!(Value::Empty.as_number(), Ok(0.0));
        assert_eq!(Value::Empty.as_bool(), Ok(false));
        assert_eq!(Value::Empty.as_text(), "");
    }

    #[test]
    fn text_does_not_silently_become_a_number() {
        assert_eq!(Value::Text("5".into()).as_number(), Err(ErrorKind::Value));
        assert_eq!(Value::Text("x".into()).as_bool(), Err(ErrorKind::Value));
    }

    #[test]
    fn booleans_coerce_to_one_and_zero() {
        assert_eq!(Value::Bool(true).as_number(), Ok(1.0));
        assert_eq!(Value::Bool(false).as_number(), Ok(0.0));
    }

    #[test]
    fn errors_propagate_through_coercion() {
        assert_eq!(Value::Error(ErrorKind::Div0).as_number(), Err(ErrorKind::Div0));
        assert_eq!(Value::Error(ErrorKind::Cycle).as_text(), "#CYCLE!");
    }

    #[test]
    fn cross_type_ordering_follows_spreadsheet_convention() {
        let num = Value::Number(1.0);
        let text = Value::Text("1".into());
        let bool_v = Value::Bool(true);
        assert_eq!(num.compare(&text), Ok(Ordering::Less));
        assert_eq!(text.compare(&bool_v), Ok(Ordering::Less));
        assert_ne!(num.compare(&text), Ok(Ordering::Equal));
        assert_eq!(bool_v.compare(&Value::Number(1.0)), Ok(Ordering::Greater));
    }

    #[test]
    fn empty_compares_equal_to_numeric_and_text_zero() {
        assert_eq!(Value::Empty.compare(&Value::Number(0.0)), Ok(Ordering::Equal));
        assert_eq!(Value::Empty.compare(&Value::Text(String::new())), Ok(Ordering::Equal));
        assert_eq!(Value::Empty.compare(&Value::Bool(false)), Ok(Ordering::Equal));
        assert_ne!(Value::Empty.compare(&Value::Number(1.0)), Ok(Ordering::Equal));
    }

    #[test]
    fn text_comparison_is_case_insensitive() {
        assert_eq!(Value::Text("a".into()).compare(&Value::Text("A".into())), Ok(Ordering::Equal));
    }

    #[test]
    fn comparison_propagates_the_left_error() {
        let err = Value::Error(ErrorKind::NA);
        assert_eq!(err.compare(&Value::Number(1.0)), Err(ErrorKind::NA));
        assert_eq!(Value::Number(1.0).compare(&err), Err(ErrorKind::NA));
    }

    #[test]
    fn numbers_render_at_fifteen_significant_digits() {
        assert_eq!(format_number(1.0), "1");
        assert_eq!(format_number(-2.5), "-2.5");
        assert_eq!(format_number(0.0), "0");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(0.1 + 0.2), "0.3");
        assert_eq!(format_number(1.0 / 3.0), "0.333333333333333");
        assert_eq!(format_number(1234567890123456.0), "1234567890123456");
        assert_eq!(format_number(1.0), "1");
    }

    #[test]
    fn extreme_magnitudes_use_scientific_notation() {
        assert_eq!(format_number(1e21), "1e21");
        assert_eq!(format_number(2.5e-10), "2.5e-10");
        assert_eq!(format_number(1.5e300), "1.5e300");
    }

    #[test]
    fn non_finite_values_never_become_numbers() {
        assert_eq!(Value::finite_number(f64::NAN), Value::Error(ErrorKind::Num));
        assert_eq!(Value::finite_number(f64::INFINITY), Value::Error(ErrorKind::Num));
        assert_eq!(Value::finite_number(1.0), Value::Number(1.0));
    }
}
