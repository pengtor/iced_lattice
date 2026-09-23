use super::*;

// Spreadsheet epoch 1899-12-30; Lattice omits Excel's phantom 1900-02-29
const EPOCH: i64 = -25_569;

const UNIX_EPOCH_SERIAL: i64 = 25_569;

// Largest supported serial: 9999-12-31
const MAX_SERIAL: i64 = 2_958_465;

pub(super) fn dispatch(
    func: FuncId,
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
) -> Option<Value> {
    Some(match func {
        FuncId::Today => today(),
        FuncId::Now => now(),
        FuncId::Date => date(args, source, host),
        FuncId::Year => component(args, source, host, Component::Year),
        FuncId::Month => component(args, source, host, Component::Month),
        FuncId::Day => component(args, source, host, Component::Day),
        FuncId::Weekday => weekday(args, source, host),
        FuncId::DateDif => datedif(args, source, host),
        _ => return None,
    })
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = (if z >= 0 { z } else { z - 146_096 }) / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn serial_from_civil(y: i64, m: i64, d: i64) -> i64 {
    days_from_civil(y, m, d) - EPOCH
}

fn civil_from_serial(serial: i64) -> (i64, i64, i64) {
    civil_from_days(serial + EPOCH)
}

// Date part truncates toward zero; negative serials rejected before
fn serial_days(serial: f64) -> Result<i64, ErrorKind> {
    if !serial.is_finite() || serial < 0.0 {
        return Err(ErrorKind::Num);
    }
    let days = serial.trunc();
    if days > MAX_SERIAL as f64 {
        return Err(ErrorKind::Num);
    }
    Ok(days as i64)
}

fn in_range(serial: i128) -> Option<i64> {
    if (0..=MAX_SERIAL as i128).contains(&serial) {
        Some(serial as i64)
    } else {
        None
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(y) => 29,
        2 => 28,
        _ => unreachable!("month is always 1..=12"),
    }
}

fn add_months(y: i64, m: i64, d: i64, months: i64) -> (i64, i64, i64) {
    let total = y * 12 + (m - 1) + months;
    let ny = total.div_euclid(12);
    let nm = total.rem_euclid(12) + 1;
    (ny, nm, d.min(days_in_month(ny, nm)))
}

#[derive(Clone, Copy)]
enum Component {
    Year,
    Month,
    Day,
}

fn component(args: &[Operand], source: &dyn ValueSource, host: CellRef, which: Component) -> Value {
    let serial = match number_arg(args, 0, source, host) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let days = match serial_days(serial) {
        Ok(days) => days,
        Err(kind) => return Value::Error(kind),
    };
    let (y, m, d) = civil_from_serial(days);
    let value = match which {
        Component::Year => y,
        Component::Month => m,
        Component::Day => d,
    };
    Value::finite_number(value as f64)
}

fn date(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let year = match count_arg(args, 0, source, host) {
        Ok(year) => year,
        Err(kind) => return Value::Error(kind),
    };
    let month = match count_arg(args, 1, source, host) {
        Ok(month) => month,
        Err(kind) => return Value::Error(kind),
    };
    let day = match count_arg(args, 2, source, host) {
        Ok(day) => day,
        Err(kind) => return Value::Error(kind),
    };

    if !(1900..=9999).contains(&year) {
        return Value::Error(ErrorKind::Num);
    }

    let total_months = (year as i128) * 12 + (month as i128 - 1);
    let normal_year = total_months.div_euclid(12);
    let normal_month = total_months.rem_euclid(12) + 1;
    if !(1899..=9999).contains(&normal_year) {
        return Value::Error(ErrorKind::Num);
    }

    let base = serial_from_civil(normal_year as i64, normal_month as i64, 1);
    let serial = base as i128 + (day as i128 - 1);
    match in_range(serial) {
        Some(serial) => Value::finite_number(serial as f64),
        None => Value::Error(ErrorKind::Num),
    }
}

fn weekday(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let serial = match number_arg(args, 0, source, host) {
        Ok(serial) => serial,
        Err(kind) => return Value::Error(kind),
    };
    let days = match serial_days(serial) {
        Ok(days) => days,
        Err(kind) => return Value::Error(kind),
    };
    let kind = match opt_count_arg(args, 1, source, host, 1) {
        Ok(kind) => kind,
        Err(kind) => return Value::Error(kind),
    };

    // Serial 2 (1900-01-01) was Monday, so Monday is index 0
    let monday_index = (days - 2).rem_euclid(7);
    let (first_index, base) = match kind {
        1 => (6, 1), // Sunday == 1
        2 => (0, 1), // Monday == 1
        3 => (0, 0), // Monday == 0
        _ => return Value::Error(ErrorKind::Num),
    };
    Value::finite_number(((monday_index - first_index).rem_euclid(7) + base) as f64)
}

fn datedif(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let start = match number_arg(args, 0, source, host).and_then(serial_days) {
        Ok(days) => days,
        Err(kind) => return Value::Error(kind),
    };
    let end = match number_arg(args, 1, source, host).and_then(serial_days) {
        Ok(days) => days,
        Err(kind) => return Value::Error(kind),
    };
    let unit = match text_arg(args, 2, source, host) {
        Ok(unit) => unit.to_ascii_uppercase(),
        Err(kind) => return Value::Error(kind),
    };
    if start > end {
        return Value::Error(ErrorKind::Num);
    }

    let (start_year, start_month, start_day) = civil_from_serial(start);
    let (end_year, end_month, end_day) = civil_from_serial(end);

    // Whole years: subtract one if anniversary not yet reached
    let mut years = end_year - start_year;
    if (end_month, end_day) < (start_month, start_day) {
        years -= 1;
    }
    // Whole months: subtract one if end day precedes start day
    let mut months = (end_year - start_year) * 12 + (end_month - start_month);
    if end_day < start_day {
        months -= 1;
    }

    let value = match unit.as_str() {
        "Y" => years,
        "M" => months,
        "D" => end - start,
        "YM" => months - 12 * years, // always 0..=11
        "MD" => {
            // Advance start by counted months, then report leftover days
            let anchor = add_months(start_year, start_month, start_day, months);
            end - serial_from_civil(anchor.0, anchor.1, anchor.2)
        }
        "YD" => {
            // Project start month/day into end year, clamped; else previous year
            let day = start_day.min(days_in_month(end_year, start_month));
            let anchor = serial_from_civil(end_year, start_month, day);
            if anchor <= end {
                end - anchor
            } else {
                let day = start_day.min(days_in_month(end_year - 1, start_month));
                end - serial_from_civil(end_year - 1, start_month, day)
            }
        }
        _ => return Value::Error(ErrorKind::Num),
    };
    Value::finite_number(value as f64)
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn today() -> Value {
    let days = unix_seconds() / 86_400;
    Value::finite_number((UNIX_EPOCH_SERIAL + days as i64) as f64)
}

fn now() -> Value {
    let seconds = unix_seconds();
    let days = seconds / 86_400;
    let fraction = (seconds % 86_400) as f64 / 86_400.0;
    Value::finite_number(UNIX_EPOCH_SERIAL as f64 + days as f64 + fraction)
}

#[cfg(test)]
mod tests {
    use crate::functions::test_support::{assert_eq_value, assert_error, eval, eval_at};
    use crate::{ErrorKind, Value};

    fn number(src: &str) -> f64 {
        match eval(src) {
            Value::Number(n) => n,
            other => panic!("{src}: expected a number, got {other:?}"),
        }
    }

    #[test]
    fn date_produces_the_documented_serials() {
        assert_eq_value("=DATE(1900, 1, 1)", Value::Number(2.0));
        assert_eq_value("=DATE(1900, 3, 1)", Value::Number(61.0));
        assert_eq_value("=DATE(1970, 1, 1)", Value::Number(25_569.0));
        assert_eq_value("=DATE(2024, 1, 1)", Value::Number(45_292.0));
        assert_eq_value("=DATE(2024, 12, 31)", Value::Number(45_657.0));
        assert_eq_value("=DATE(9999, 12, 31)", Value::Number(2_958_465.0));
    }

    #[test]
    fn year_month_and_day_read_the_serial_back() {
        // The epoch itself, 1899-12-30.
        assert_eq_value("=YEAR(0)", Value::Number(1899.0));
        assert_eq_value("=MONTH(0)", Value::Number(12.0));
        assert_eq_value("=DAY(0)", Value::Number(30.0));
        assert_eq_value("=YEAR(61)", Value::Number(1900.0));
        assert_eq_value("=MONTH(61)", Value::Number(3.0));
        assert_eq_value("=DAY(61)", Value::Number(1.0));
        assert_eq_value("=YEAR(45292)", Value::Number(2024.0));
        assert_eq_value("=MONTH(45292)", Value::Number(1.0));
        assert_eq_value("=DAY(45292)", Value::Number(1.0));
        assert_eq_value("=YEAR(25569)", Value::Number(1970.0));
    }

    #[test]
    fn a_fractional_serial_is_truncated_toward_zero_for_the_date_part() {
        assert_eq_value("=YEAR(45292.75)", Value::Number(2024.0));
        assert_eq_value("=MONTH(45292.75)", Value::Number(1.0));
        assert_eq_value("=DAY(45292.75)", Value::Number(1.0));
    }

    #[test]
    fn date_round_trips_through_year_month_day() {
        let cases = [
            ("=DATE(1900, 3, 1)", 1900.0, 3.0, 1.0),
            ("=DATE(2000, 2, 29)", 2000.0, 2.0, 29.0),
            ("=DATE(2023, 12, 31)", 2023.0, 12.0, 31.0),
            ("=DATE(2024, 1, 1)", 2024.0, 1.0, 1.0),
            ("=DATE(9999, 12, 31)", 9999.0, 12.0, 31.0),
        ];
        for (src, y, m, d) in cases {
            let serial = number(src);
            assert_eq!(number(&format!("=DATE({y}, {m}, {d})")), serial, "{src}");
            assert_eq!(number(&format!("=YEAR({serial})")), y, "{src}");
            assert_eq!(number(&format!("=MONTH({serial})")), m, "{src}");
            assert_eq!(number(&format!("=DAY({serial})")), d, "{src}");
        }
    }

    #[test]
    fn date_normalises_out_of_range_months_and_days() {
        assert_eq_value("=DATE(2024, 13, 1)", eval("=DATE(2025, 1, 1)"));
        assert_eq_value("=DATE(2024, 0, 1)", eval("=DATE(2023, 12, 1)"));
        assert_eq_value("=DATE(2024, 1, 32)", eval("=DATE(2024, 2, 1)"));
        assert_eq_value("=DATE(2024, 2, -1)", eval("=DATE(2024, 1, 30)"));
        // A negative month can walk the year back too.
        assert_eq_value("=DATE(2024, -11, 1)", eval("=DATE(2023, 1, 1)"));
    }

    #[test]
    fn dates_outside_the_supported_range_are_num_errors() {
        assert_error("=DATE(1899, 12, 30)", ErrorKind::Num);
        assert_error("=DATE(10000, 1, 1)", ErrorKind::Num);
        // Normalisation pushes the year below 1900.
        assert_error("=DATE(1900, 0, 1)", ErrorKind::Num);
        // The resulting serial is negative.
        assert_error("=DATE(1900, 1, -2)", ErrorKind::Num);
        // Beyond 9999-12-31.
        assert_error("=DATE(9999, 12, 32)", ErrorKind::Num);
        assert_error("=DATE(2024, 1, \"x\")", ErrorKind::Value);
    }

    #[test]
    fn serial_inputs_outside_the_range_are_num_errors() {
        assert_error("=YEAR(-1)", ErrorKind::Num);
        assert_error("=MONTH(-0.5)", ErrorKind::Num);
        assert_error("=DAY(2958466)", ErrorKind::Num);
        assert_error("=WEEKDAY(-1)", ErrorKind::Num);
        assert_error("=YEAR(\"x\")", ErrorKind::Value);
    }

    #[test]
    fn weekday_numbers_the_day_for_each_return_type() {
        // 2024-01-01 was a Monday, serial 45292.
        assert_eq_value("=WEEKDAY(45292)", Value::Number(2.0)); // default type 1
        assert_eq_value("=WEEKDAY(45292, 1)", Value::Number(2.0)); // Monday == 2
        assert_eq_value("=WEEKDAY(45292, 2)", Value::Number(1.0)); // Monday == 1
        assert_eq_value("=WEEKDAY(45292, 3)", Value::Number(0.0)); // Monday == 0
        // 2024-01-07 was a Sunday, serial 45298.
        assert_eq_value("=WEEKDAY(45298, 1)", Value::Number(1.0));
        assert_eq_value("=WEEKDAY(45298, 2)", Value::Number(7.0));
        assert_eq_value("=WEEKDAY(45298, 3)", Value::Number(6.0));
    }

    #[test]
    fn weekday_rejects_unknown_return_types_and_bad_numbers() {
        assert_error("=WEEKDAY(45292, 4)", ErrorKind::Num);
        assert_error("=WEEKDAY(45292, 0)", ErrorKind::Num);
        assert_error("=WEEKDAY(45292, \"x\")", ErrorKind::Value);
    }

    #[test]
    fn datedif_days_months_and_years_come_from_the_calendar() {
        assert_eq_value("=DATEDIF(DATE(2024, 1, 1), DATE(2024, 3, 15), \"D\")", Value::Number(74.0));
        assert_eq_value("=DATEDIF(DATE(2024, 1, 15), DATE(2024, 4, 14), \"M\")", Value::Number(2.0));
        assert_eq_value("=DATEDIF(DATE(2024, 1, 15), DATE(2024, 4, 15), \"M\")", Value::Number(3.0));
        // One day short of the fourth anniversary.
        assert_eq_value("=DATEDIF(DATE(2020, 3, 15), DATE(2024, 3, 14), \"Y\")", Value::Number(3.0));
        assert_eq_value("=DATEDIF(DATE(2020, 3, 15), DATE(2024, 3, 15), \"Y\")", Value::Number(4.0));
    }

    #[test]
    fn datedif_ym_md_and_yd_drop_the_coarser_components() {
        // One whole month, with 28 days left over
        assert_eq_value("=DATEDIF(DATE(2024, 1, 15), DATE(2024, 3, 14), \"YM\")", Value::Number(1.0));
        assert_eq_value("=DATEDIF(DATE(2024, 1, 15), DATE(2024, 3, 14), \"MD\")", Value::Number(28.0));
        // Across a year boundary YM is still 0..=11.
        assert_eq_value("=DATEDIF(DATE(2023, 11, 30), DATE(2024, 2, 29), \"YM\")", Value::Number(2.0));
        // Ignoring years: 2024-01-01 to 2025-03-15 is 73 days.
        assert_eq_value("=DATEDIF(DATE(2024, 1, 1), DATE(2025, 3, 15), \"YD\")", Value::Number(73.0));
    }

    #[test]
    fn datedif_units_are_case_insensitive() {
        assert_eq_value("=DATEDIF(DATE(2024, 1, 1), DATE(2024, 3, 15), \"d\")", Value::Number(74.0));
        assert_eq_value("=DATEDIF(DATE(2024, 1, 1), DATE(2024, 3, 15), \"ym\")", Value::Number(2.0));
    }

    #[test]
    fn datedif_rejects_reversed_dates_and_unknown_units() {
        assert_error("=DATEDIF(DATE(2024, 3, 1), DATE(2024, 1, 1), \"D\")", ErrorKind::Num);
        assert_error("=DATEDIF(DATE(2024, 1, 1), DATE(2024, 1, 1), \"Q\")", ErrorKind::Num);
        assert_error("=DATEDIF(-1, DATE(2024, 1, 1), \"D\")", ErrorKind::Num);
        // Number renders as text; not a known unit
        assert_error("=DATEDIF(DATE(2024, 1, 1), DATE(2024, 1, 1), 3)", ErrorKind::Num);
    }

    #[test]
    fn today_and_now_have_the_right_shape_rather_than_a_fixed_value() {
        // TODAY is a whole number near the present day
        let today = number("=TODAY()");
        assert_eq!(today.fract(), 0.0, "TODAY must be a whole serial");
        assert!(today > 40_000.0 && today < 60_000.0, "TODAY = {today} is implausible");

        // NOW within a day of TODAY; may roll over midnight
        let now = number("=NOW()");
        assert!((now - today).abs() < 1.0, "NOW = {now} is not within a day of TODAY = {today}");
        assert!(now.fract() >= 0.0);
    }

    #[test]
    fn a_stored_serial_reads_back_through_the_range() {
        let cells = [("A1", Value::Number(45_292.0))];
        assert_eq!(eval_at("=YEAR(A1)", &cells, "B1"), Value::Number(2024.0));
    }
}
