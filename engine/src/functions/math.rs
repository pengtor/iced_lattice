
use super::*;

pub(super) fn dispatch(
    func: FuncId,
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
) -> Option<Value> {
    Some(match func {
        FuncId::Round => round_to(args, source, host, Rounding::HalfAwayFromZero),
        FuncId::RoundUp => round_to(args, source, host, Rounding::Up),
        FuncId::RoundDown | FuncId::Trunc => round_to(args, source, host, Rounding::Down),
        FuncId::Abs => unary(args, source, host, f64::abs),
        FuncId::Sqrt => sqrt(args, source, host),
        FuncId::Power => power(args, source, host),
        FuncId::Mod => modulo(args, source, host),
        FuncId::Int => unary(args, source, host, f64::floor),
        FuncId::Ceiling => ceiling_floor(args, source, host, true),
        FuncId::Floor => ceiling_floor(args, source, host, false),
        FuncId::Sign => unary(args, source, host, |n| {
            if n > 0.0 {
                1.0
            } else if n < 0.0 {
                -1.0
            } else {
                0.0
            }
        }),
        _ => return None,
    })
}

fn unary(args: &[Operand], source: &dyn ValueSource, host: CellRef, f: fn(f64) -> f64) -> Value {
    match number_arg(args, 0, source, host) {
        Ok(n) => Value::finite_number(f(n)),
        Err(kind) => Value::Error(kind),
    }
}

// clamp so a wild digit count saturates instead of wrapping
fn round_to(
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
    mode: Rounding,
) -> Value {
    let n = match number_arg(args, 0, source, host) {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    let digits = match opt_count_arg(args, 1, source, host, 0) {
        Ok(digits) => digits,
        Err(kind) => return Value::Error(kind),
    };
    let places = digits.clamp(i32::MIN as i64, i32::MAX as i64) as i32;
    Value::finite_number(round_decimal(n, places, mode))
}

fn sqrt(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    match number_arg(args, 0, source, host) {
        Ok(n) if n < 0.0 => Value::Error(ErrorKind::Num),
        Ok(n) => Value::finite_number(n.sqrt()),
        Err(kind) => Value::Error(kind),
    }
}

// like the ^ operator; overflow or non-real is #NUM!
fn power(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let base = match number_arg(args, 0, source, host) {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    let exponent = match number_arg(args, 1, source, host) {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    Value::finite_number(base.powf(exponent))
}

// remainder takes the divisor's sign, unlike Rust's %
fn modulo(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let n = match number_arg(args, 0, source, host) {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    let d = match number_arg(args, 1, source, host) {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    if d == 0.0 {
        return Value::Error(ErrorKind::Div0);
    }
    Value::finite_number(n - d * (n / d).floor())
}

fn ceiling_floor(
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
    up: bool,
) -> Value {
    let n = match number_arg(args, 0, source, host) {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    let significance = match opt_number_arg(args, 1, source, host, 1.0) {
        Ok(n) => n,
        Err(kind) => return Value::Error(kind),
    };
    let step = significance.abs();
    if step == 0.0 {
        // zero significance is zero, not a division error
        return Value::Number(0.0);
    }
    let multiple = n / step;
    let rounded = if up { multiple.ceil() } else { multiple.floor() };
    Value::finite_number(rounded * step)
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use crate::{ErrorKind, Value};

    #[test]
    fn round_uses_decimal_digits_and_half_away_from_zero() {
        assert_eq!(eval("=ROUND(2.675, 2)"), Value::Number(2.68));
        assert_eq!(eval("=ROUND(1.005, 2)"), Value::Number(1.01));
        assert_eq!(eval("=ROUND(2.5, 0)"), Value::Number(3.0));
        assert_eq!(eval("=ROUND(-2.5, 0)"), Value::Number(-3.0));
        assert_eq!(eval("=ROUND(2.4)"), Value::Number(2.0));
    }

    #[test]
    fn round_accepts_negative_digit_counts() {
        assert_eq!(eval("=ROUND(1234, -2)"), Value::Number(1200.0));
        assert_eq!(eval("=ROUND(1250, -2)"), Value::Number(1300.0));
    }

    #[test]
    fn roundup_and_rounddown_leave_zero_and_ties_alone() {
        assert_eq!(eval("=ROUNDUP(2.1, 0)"), Value::Number(3.0));
        assert_eq!(eval("=ROUNDUP(-2.1, 0)"), Value::Number(-3.0));
        assert_eq!(eval("=ROUNDUP(2.0, 0)"), Value::Number(2.0));
        assert_eq!(eval("=ROUNDDOWN(-2.9, 0)"), Value::Number(-2.0));
        assert_eq!(eval("=ROUNDDOWN(2.9, 0)"), Value::Number(2.0));
        assert_eq!(eval("=ROUNDDOWN(2.0, 0)"), Value::Number(2.0));
    }

    #[test]
    fn trunc_differs_from_int_at_negative_arguments() {
        assert_eq!(eval("=TRUNC(2.9)"), Value::Number(2.0));
        assert_eq!(eval("=TRUNC(-1.5)"), Value::Number(-1.0));
        assert_eq!(eval("=INT(-1.5)"), Value::Number(-2.0));
        assert_eq!(eval("=INT(2.9)"), Value::Number(2.0));
        assert_eq!(eval("=TRUNC(2.9, 0)"), Value::Number(2.0));
    }

    #[test]
    fn round_family_rejects_text_and_saturates_a_wild_digit_count() {
        assert_error("=ROUND(\"2.5\", 0)", ErrorKind::Value);
        assert_error("=ROUNDDOWN(\"x\")", ErrorKind::Value);
        assert_eq!(eval("=ROUND(1234, 5000000000)"), Value::Number(1234.0));
        assert_eq!(eval("=ROUND(1234, -5000000000)"), Value::Number(0.0));
    }

    #[test]
    fn abs_is_the_magnitude_and_normalizes_negative_zero() {
        assert_eq!(eval("=ABS(-3)"), Value::Number(3.0));
        assert_eq!(eval("=ABS(3)"), Value::Number(3.0));
        assert_eq!(eval("=ABS(-0)"), Value::Number(0.0));
        assert_error("=ABS(\"x\")", ErrorKind::Value);
    }

    #[test]
    fn sqrt_of_a_negative_is_a_num_error() {
        assert_eq!(eval("=SQRT(9)"), Value::Number(3.0));
        assert_eq!(eval("=SQRT(0)"), Value::Number(0.0));
        assert_error("=SQRT(-1)", ErrorKind::Num);
    }

    #[test]
    fn power_matches_the_caret_operator_and_guards_overflow() {
        assert_eq!(eval("=POWER(2, 10)"), Value::Number(1024.0));
        assert_eq!(eval("=POWER(9, 0.5)"), Value::Number(3.0));
        assert_error("=POWER(-8, 0.5)", ErrorKind::Num);
        assert_error("=POWER(10, 400)", ErrorKind::Num);
        assert_error("=POWER(\"x\", 2)", ErrorKind::Value);
    }

    #[test]
    fn mod_takes_the_sign_of_the_divisor() {
        assert_eq!(eval("=MOD(-3, 2)"), Value::Number(1.0));
        assert_eq!(eval("=MOD(3, -2)"), Value::Number(-1.0));
        assert_eq!(eval("=MOD(3, 2)"), Value::Number(1.0));
        assert_error("=MOD(1, 0)", ErrorKind::Div0);
    }

    #[test]
    fn ceiling_and_floor_round_to_a_multiple_without_sign_errors() {
        assert_eq!(eval("=CEILING(2.5, 1)"), Value::Number(3.0));
        assert_eq!(eval("=FLOOR(2.5, 1)"), Value::Number(2.0));
        assert_eq!(eval("=CEILING(-2.5, 1)"), Value::Number(-2.0));
        assert_eq!(eval("=FLOOR(-2.5, 1)"), Value::Number(-3.0));
        assert_eq!(eval("=CEILING(2.1)"), Value::Number(3.0));
        assert_eq!(eval("=FLOOR(2.9)"), Value::Number(2.0));
        assert_eq!(eval("=CEILING(7, 5)"), Value::Number(10.0));
        assert_eq!(eval("=FLOOR(7, 5)"), Value::Number(5.0));
        assert_eq!(eval("=CEILING(2.5, 0)"), Value::Number(0.0));
        assert_eq!(eval("=FLOOR(2.5, 0)"), Value::Number(0.0));
    }

    #[test]
    fn ceiling_and_floor_reject_text() {
        assert_error("=CEILING(\"x\")", ErrorKind::Value);
        assert_error("=FLOOR(1, \"x\")", ErrorKind::Value);
    }

    #[test]
    fn sign_reports_minus_one_zero_or_one() {
        assert_eq!(eval("=SIGN(-4)"), Value::Number(-1.0));
        assert_eq!(eval("=SIGN(0)"), Value::Number(0.0));
        assert_eq!(eval("=SIGN(3)"), Value::Number(1.0));
        assert_error("=SIGN(\"x\")", ErrorKind::Value);
    }
}
