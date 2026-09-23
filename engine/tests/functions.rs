
use std::collections::HashMap;

use engine::{compile_source, evaluate, CellRef, ErrorKind, Value};

fn sheet(cells: &[(&str, Value)]) -> HashMap<CellRef, Value> {
    let mut map = HashMap::new();
    for (a1, value) in cells {
        map.insert(CellRef::parse_a1(a1).unwrap(), value.clone());
    }
    map
}

fn at(src: &str, cells: &[(&str, Value)]) -> Value {
    let program = compile_source(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    evaluate(&program, &sheet(cells), CellRef::parse_a1("Z100").unwrap())
}

fn e(src: &str) -> Value {
    at(src, &[])
}

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn t(x: &str) -> Value {
    Value::Text(x.to_string())
}

fn bad(kind: ErrorKind) -> Value {
    Value::Error(kind)
}

macro_rules! check {
    ($($src:literal => $expected:expr),* $(,)?) => {
        $( assert_eq!(e($src), $expected, "{}", $src); )*
    };
}

macro_rules! check_on {
    ($cells:expr, $($src:literal => $expected:expr),* $(,)?) => {
        $( assert_eq!(at($src, $cells), $expected, "{}", $src); )*
    };
}

#[test]
fn logic_functions_coerce_scalars_and_ignore_text_inside_ranges() {
    check! {
        "=AND(TRUE, 1)" => Value::Bool(true),
        "=AND(TRUE, 0)" => Value::Bool(false),
        "=OR(FALSE, 1)" => Value::Bool(true),
        "=OR(FALSE, 0)" => Value::Bool(false),
        "=XOR(TRUE, FALSE)" => Value::Bool(true),
        "=XOR(TRUE, TRUE)" => Value::Bool(false),
        "=XOR(TRUE, FALSE, TRUE)" => Value::Bool(false),
        "=NOT(0)" => Value::Bool(true),
        "=NOT(FALSE)" => Value::Bool(true),
        "=NOT(\"x\")" => bad(ErrorKind::Value),
        "=AND(\"x\")" => bad(ErrorKind::Value),
        "=AND(#N/A, TRUE)" => bad(ErrorKind::NA),
    }
    let cells = [("A1", Value::Bool(true)), ("A2", t("text")), ("A3", n(0.0))];
    check_on!(
        &cells,
        "=AND(A1:A3)" => Value::Bool(false),
        "=OR(A1:A3)" => Value::Bool(true),
    );
}

#[test]
fn type_predicates_report_instead_of_propagating() {
    check! {
        "=ISBLANK(Z9)" => Value::Bool(true),
        "=ISBLANK(1)" => Value::Bool(false),
        "=ISNUMBER(1)" => Value::Bool(true),
        "=ISNUMBER(\"1\")" => Value::Bool(false),
        "=ISTEXT(\"a\")" => Value::Bool(true),
        "=ISTEXT(1)" => Value::Bool(false),
        "=ISERROR(1/0)" => Value::Bool(true),
        "=ISERROR(1)" => Value::Bool(false),
        "=ISNA(#N/A)" => Value::Bool(true),
        "=ISNA(1/0)" => Value::Bool(false),
    }
}

#[test]
fn rounding_is_decimal_not_binary() {
    check! {
        // The three cases binary scaling gets wrong.
        "=ROUND(2.675, 2)" => n(2.68),
        "=ROUND(1.005, 2)" => n(1.01),
        "=ROUND(0.5, 0)" => n(1.0),
        "=ROUND(-0.5, 0)" => n(-1.0),
        "=ROUND(2.4, 0)" => n(2.0),
        "=ROUND(1234, -2)" => n(1200.0),
        "=ROUND(1250, -2)" => n(1300.0),
        "=ROUND(2.5)" => n(3.0),
        "=ROUNDUP(2.1, 0)" => n(3.0),
        "=ROUNDUP(-2.1, 0)" => n(-3.0),
        "=ROUNDDOWN(2.9, 0)" => n(2.0),
        "=ROUNDDOWN(-2.9, 0)" => n(-2.0),
        "=TRUNC(2.9)" => n(2.0),
        "=TRUNC(-2.9)" => n(-2.0),
        "=INT(2.9)" => n(2.0),
        "=INT(-1.5)" => n(-2.0),
    }
}

#[test]
fn math_edge_cases_are_error_values_not_panics() {
    check! {
        "=SQRT(-1)" => bad(ErrorKind::Num),
        "=SQRT(0)" => n(0.0),
        "=SQRT(9)" => n(3.0),
        "=POWER(-8, 0.5)" => bad(ErrorKind::Num),
        "=POWER(2, 10)" => n(1024.0),
        "=MOD(1, 0)" => bad(ErrorKind::Div0),
        // The result takes the divisor's sign.
        "=MOD(-3, 2)" => n(1.0),
        "=MOD(3, -2)" => n(-1.0),
        "=MOD(7, 3)" => n(1.0),
        "=ABS(-3)" => n(3.0),
        "=SIGN(-3)" => n(-1.0),
        "=SIGN(0)" => n(0.0),
        "=SIGN(3)" => n(1.0),
        "=ABS(\"3\")" => bad(ErrorKind::Value),
        "=ROUND(\"2.5\", 0)" => bad(ErrorKind::Value),
        // CEILING/FLOOR follow .MATH rules: toward +/- infinity, never #NUM!.
        "=CEILING(2.5, 1)" => n(3.0),
        "=CEILING(-2.5, 1)" => n(-2.0),
        "=FLOOR(2.5, 1)" => n(2.0),
        "=FLOOR(-2.5, 1)" => n(-3.0),
        "=CEILING(2.5, 0)" => n(0.0),
        "=FLOOR(7, 3)" => n(6.0),
        "=CEILING(7, 3)" => n(9.0),
    }
}

#[test]
fn text_functions_count_characters_not_bytes() {
    check! {
        "=LEN(\"héllo\")" => n(5.0),
        "=LEN(1.5)" => n(3.0),
        "=LEN(\"\")" => n(0.0),
        "=UPPER(\"aB\")" => t("AB"),
        "=LOWER(\"aB\")" => t("ab"),
        "=TRIM(\"  a   b  \")" => t("a b"),
        "=LEFT(\"abc\", 2)" => t("ab"),
        "=LEFT(\"abc\")" => t("a"),
        "=LEFT(\"abc\", 99)" => t("abc"),
        "=LEFT(\"abc\", -1)" => bad(ErrorKind::Value),
        "=RIGHT(\"abc\", 2)" => t("bc"),
        "=RIGHT(\"abc\")" => t("c"),
        "=MID(\"héllo\", 2, 1)" => t("é"),
        "=MID(\"abc\", 2, 2)" => t("bc"),
        "=MID(\"abc\", 0, 1)" => bad(ErrorKind::Value),
        "=MID(\"abc\", 1, -1)" => bad(ErrorKind::Value),
        "=MID(\"abc\", 99, 1)" => t(""),
    }
}

#[test]
fn find_is_case_sensitive_and_search_is_not() {
    check! {
        "=FIND(\"b\", \"abc\")" => n(2.0),
        "=FIND(\"B\", \"abc\")" => bad(ErrorKind::Value),
        "=SEARCH(\"B\", \"abc\")" => n(2.0),
        "=SEARCH(\"b*\", \"abc\")" => n(2.0),
        "=SEARCH(\"a?c\", \"abcd\")" => n(1.0),
        "=FIND(\"z\", \"abc\")" => bad(ErrorKind::Value),
        "=SEARCH(\"z\", \"abc\")" => bad(ErrorKind::Value),
        "=FIND(\"b\", \"abc\", 3)" => bad(ErrorKind::Value),
        "=FIND(\"\", \"abc\")" => n(1.0),
    }
}

#[test]
fn substitutions_and_replacement_count_from_one() {
    check! {
        "=SUBSTITUTE(\"a-b-c\", \"-\", \"+\")" => t("a+b+c"),
        "=SUBSTITUTE(\"a-b-c\", \"-\", \"+\", 2)" => t("a-b+c"),
        "=SUBSTITUTE(\"a-b-c\", \"-\", \"+\", 9)" => t("a-b-c"),
        "=SUBSTITUTE(\"abc\", \"\", \"x\")" => t("abc"),
        "=REPLACE(\"abcdef\", 2, 3, \"X\")" => t("aXef"),
        "=REPLACE(\"abcdef\", 1, 0, \"X\")" => t("Xabcdef"),
        "=REPLACE(\"abcdef\", 0, 1, \"X\")" => bad(ErrorKind::Value),
        "=REPLACE(\"abcdef\", 1, -1, \"X\")" => bad(ErrorKind::Value),
    }
}

#[test]
fn text_formats_numbers_and_value_parses_them_back() {
    check! {
        "=TEXT(1234.5, \"#,##0.00\")" => t("1,234.50"),
        "=TEXT(1234.5, \"0\")" => t("1235"),
        "=TEXT(0.125, \"0.0%\")" => t("12.5%"),
        "=TEXT(2.5, \"0\")" => t("3"),
        "=TEXT(1, \"0.00\")" => t("1.00"),
        "=TEXT(-5, \"0\")" => t("-5"),
        "=TEXT(\"x\", \"0\")" => bad(ErrorKind::Value),
        "=VALUE(\"42\")" => n(42.0),
        "=VALUE(\"$1,234.50\")" => n(1234.5),
        "=VALUE(\"(1.5)\")" => n(-1.5),
        "=VALUE(\"50%\")" => n(0.5),
        "=VALUE(\" 1.5e3 \")" => n(1500.0),
        "=VALUE(\"-2.5\")" => n(-2.5),
        "=VALUE(7)" => n(7.0),
        "=VALUE(\"abc\")" => bad(ErrorKind::Value),
        "=VALUE(\"\")" => bad(ErrorKind::Value),
        "=VALUE(\"1.2.3\")" => bad(ErrorKind::Value),
        "=VALUE(\"1e\")" => bad(ErrorKind::Value),
        "=VALUE(\"$\")" => bad(ErrorKind::Value),
    }
}

fn table() -> Vec<(&'static str, Value)> {
    vec![
        ("A1", n(10.0)),
        ("B1", t("ten")),
        ("C1", n(30.0)),
        ("A2", n(20.0)),
        ("B2", t("twenty")),
        ("C2", n(20.0)),
        ("A3", n(30.0)),
        ("B3", t("thirty")),
        ("C3", n(10.0)),
        ("E1", n(1.0)),
        ("F1", n(2.0)),
        ("G1", n(3.0)),
    ]
}

#[test]
fn index_takes_offsets_within_the_range() {
    let cells = table();
    check_on!(
        &cells,
        "=INDEX(A1:B3, 2, 2)" => t("twenty"),
        "=INDEX(A1:B3, 1, 1)" => n(10.0),
        "=INDEX(A1:A3, 2)" => n(20.0),
        "=INDEX(A1:C1, 2)" => t("ten"),
        "=INDEX(E1:G1, 3)" => n(3.0),
        "=INDEX(5, 1, 1)" => n(5.0),
        "=INDEX(A1:B3, 0, 1)" => bad(ErrorKind::Value),
        "=INDEX(A1:B3, 1)" => bad(ErrorKind::Value),
        "=INDEX(A1:B3, 9, 1)" => bad(ErrorKind::Ref),
        "=INDEX(A1:B3, 1, 9)" => bad(ErrorKind::Ref),
    );
}

#[test]
fn match_walks_in_order_and_reports_the_position() {
    let cells = table();
    check_on!(
        &cells,
        "=MATCH(20, A1:A3, 0)" => n(2.0),
        "=MATCH(25, A1:A3, 0)" => bad(ErrorKind::NA),
        "=MATCH(25, A1:A3, 1)" => n(2.0),
        "=MATCH(5, A1:A3, 1)" => bad(ErrorKind::NA),
        "=MATCH(25, C1:C3, -1)" => n(1.0),
        "=MATCH(5, C1:C3, -1)" => n(3.0),
        "=MATCH(99, C1:C3, -1)" => bad(ErrorKind::NA),
        "=MATCH(\"tw*\", B1:B3, 0)" => n(2.0),
        "=MATCH(1, A1:A3, 2)" => bad(ErrorKind::Value),
    );
}

#[test]
fn vlookup_and_hlookup_cover_both_matching_modes() {
    let cells = table();
    check_on!(
        &cells,
        "=VLOOKUP(20, A1:B3, 2, FALSE)" => t("twenty"),
        "=VLOOKUP(20, A1:B3, 2, 0)" => t("twenty"),
        "=VLOOKUP(25, A1:B3, 2, FALSE)" => bad(ErrorKind::NA),
        "=VLOOKUP(25, A1:B3, 2)" => t("twenty"),
        "=VLOOKUP(25, A1:B3, 2, TRUE)" => t("twenty"),
        "=VLOOKUP(5, A1:B3, 2, TRUE)" => bad(ErrorKind::NA),
        "=VLOOKUP(20, A1:B3, 3, FALSE)" => bad(ErrorKind::Ref),
        "=VLOOKUP(20, A1:B3, 0, FALSE)" => bad(ErrorKind::Ref),
        "=VLOOKUP(\"tw*\", B1:B3, 1, FALSE)" => t("twenty"),
        "=HLOOKUP(10, A1:C2, 2, FALSE)" => n(20.0),
        "=HLOOKUP(10, A1:C2, 1, FALSE)" => n(10.0),
        "=HLOOKUP(99, A1:C2, 2, FALSE)" => bad(ErrorKind::NA),
    );
}

#[test]
fn xlookup_matches_position_by_position() {
    let cells = table();
    check_on!(
        &cells,
        "=XLOOKUP(20, A1:A3, B1:B3)" => t("twenty"),
        "=XLOOKUP(99, A1:A3, B1:B3)" => bad(ErrorKind::NA),
        "=XLOOKUP(99, A1:A3, B1:B3, \"none\")" => t("none"),
        "=XLOOKUP(20, A1:A3, B1:B2)" => bad(ErrorKind::Value),
        "=XLOOKUP(20, 5, B1:B3)" => bad(ErrorKind::Value),
    );
}

#[test]
fn criteria_do_not_compare_across_types() {
    let cells = table();
    check_on!(
        &cells,
        "=COUNTIF(A1:A3, \">15\")" => n(2.0),
        "=COUNTIF(A1:A3, \">=10\")" => n(3.0),
        "=COUNTIF(A1:A3, 20)" => n(1.0),
        "=COUNTIF(A1:A3, \"20\")" => n(1.0),
        "=COUNTIF(A1:A3, \"<>20\")" => n(2.0),
        // numeric criteria must not match text, despite cross-type ordering
        "=COUNTIF(B1:B3, \">15\")" => n(0.0),
        "=COUNTIF(B1:B3, \"tw*\")" => n(1.0),
        "=COUNTIF(B1:B3, \"T*\")" => n(3.0),
        "=COUNTIF(B1:B3, \"tw?nty\")" => n(1.0),
        "=COUNTIF(B1:B3, \"zzz\")" => n(0.0),
    );
}

#[test]
fn conditional_aggregates_sum_the_matching_rows() {
    let cells = table();
    check_on!(
        &cells,
        "=SUMIF(A1:A3, \">15\")" => n(50.0),
        "=SUMIF(A1:A3, \">15\", A1:A3)" => n(50.0),
        "=SUMIF(A1:A3, \">15\", C1:C3)" => n(30.0),
        "=SUMIF(A1:A3, \">99\")" => n(0.0),
        "=AVERAGEIF(A1:A3, \">15\")" => n(25.0),
        "=AVERAGEIF(A1:A3, \">99\")" => bad(ErrorKind::Div0),
        "=SUMIFS(A1:A3, B1:B3, \"tw*\")" => n(20.0),
        "=COUNTIFS(A1:A3, \">15\", B1:B3, \"tw*\")" => n(1.0),
        "=COUNTIFS(A1:A3, \">15\", B1:B3, \"zzz\")" => n(0.0),
        "=AVERAGEIFS(A1:A3, B1:B3, \"t*\")" => n(20.0),
        "=COUNTIFS(A1:A3, \">15\", B1:B2, \"t*\")" => bad(ErrorKind::Value),
        "=SUMIF(A1:A3, \">15\", B1:B2)" => bad(ErrorKind::Value),
    );
}

#[test]
fn conditional_aggregates_propagate_errors_in_what_they_add_up() {
    let cells = [
        ("A1", n(10.0)),
        ("A2", n(20.0)),
        ("A3", n(30.0)),
        ("C1", bad(ErrorKind::NA)),
        ("C2", n(5.0)),
        ("C3", n(5.0)),
    ];
    check_on!(
        &cells,
        "=SUMIF(A1:A3, \">15\", C1:C3)" => n(10.0),
        "=SUMIF(A1:A3, \">5\", C1:C3)" => bad(ErrorKind::NA),
    );
}

#[test]
fn statistics_follow_the_aggregate_conventions() {
    let cells = [("A1", n(1.0)), ("A2", n(2.0)), ("A3", n(3.0)), ("A4", t("x"))];
    check! {
        "=MEDIAN(1, 2, 3)" => n(2.0),
        "=MEDIAN(1, 2, 3, 4)" => n(2.5),
        "=MEDIAN(1, 2)" => n(1.5),
        "=MODE(1, 2, 2, 3)" => n(2.0),
        "=MODE(1, 2, 3)" => bad(ErrorKind::NA),
        "=VAR(1, 2, 3)" => n(1.0),
        "=STDEV(1, 2, 3)" => n(1.0),
        // Empty input follows AVERAGE rather than Excel's #NUM!.
        "=MEDIAN()" => bad(ErrorKind::Div0),
        "=MODE()" => bad(ErrorKind::Div0),
        "=STDEV()" => bad(ErrorKind::Div0),
        "=VAR()" => bad(ErrorKind::Div0),
        "=VAR(1)" => bad(ErrorKind::Div0),
        "=STDEV(1)" => bad(ErrorKind::Div0),
    }
    check_on!(
        &cells,
        "=MEDIAN(A1:A4)" => n(2.0),
        "=VAR(A1:A4)" => n(1.0),
    );
}

#[test]
fn dates_are_serials_from_the_documented_epoch() {
    check! {
        "=DATE(1900, 3, 1)" => n(61.0),
        "=DATE(1970, 1, 1)" => n(25569.0),
        "=DATE(2024, 1, 1)" => n(45292.0),
        "=DATE(9999, 12, 31)" => n(2958465.0),
        "=YEAR(0)" => n(1899.0),
        "=MONTH(0)" => n(12.0),
        "=DAY(0)" => n(30.0),
        "=YEAR(45292)" => n(2024.0),
        "=MONTH(45292)" => n(1.0),
        "=DAY(45292)" => n(1.0),
        "=DATE(2024, 13, 1)" => n(45658.0),
        "=DATE(2024, 0, 1)" => n(45261.0),
        "=DATE(2024, 1, 32)" => n(45323.0),
        "=DATE(1899, 1, 1)" => bad(ErrorKind::Num),
        "=DATE(10000, 1, 1)" => bad(ErrorKind::Num),
        "=YEAR(-1)" => bad(ErrorKind::Num),
        "=MONTH(2958466)" => bad(ErrorKind::Num),
    }
}

#[test]
fn weekdays_and_datedif() {
    check! {
        "=WEEKDAY(45292)" => n(2.0),
        "=WEEKDAY(45292, 1)" => n(2.0),
        "=WEEKDAY(45292, 2)" => n(1.0),
        "=WEEKDAY(45292, 3)" => n(0.0),
        "=WEEKDAY(45292, 4)" => bad(ErrorKind::Num),
        "=DATEDIF(DATE(2020, 1, 15), DATE(2024, 3, 10), \"Y\")" => n(4.0),
        "=DATEDIF(DATE(2020, 1, 15), DATE(2024, 3, 10), \"M\")" => n(49.0),
        "=DATEDIF(DATE(2024, 1, 1), DATE(2024, 1, 31), \"D\")" => n(30.0),
        "=DATEDIF(DATE(2020, 1, 15), DATE(2024, 3, 10), \"YM\")" => n(1.0),
        "=DATEDIF(DATE(2024, 1, 15), DATE(2024, 3, 20), \"MD\")" => n(5.0),
        "=DATEDIF(DATE(2024, 1, 15), DATE(2024, 3, 20), \"YD\")" => n(65.0),
        "=DATEDIF(DATE(2020, 1, 15), DATE(2024, 3, 10), \"y\")" => n(4.0),
        "=DATEDIF(DATE(2024, 3, 1), DATE(2024, 1, 1), \"D\")" => bad(ErrorKind::Num),
        "=DATEDIF(DATE(2024, 1, 1), DATE(2024, 2, 1), \"Q\")" => bad(ErrorKind::Num),
    }
}

#[test]
fn today_and_now_read_the_clock_without_a_timer() {
    let today = match e("=TODAY()") {
        Value::Number(n) => n,
        other => panic!("TODAY() gave {other:?}"),
    };
    assert_eq!(today.fract(), 0.0, "TODAY() is a whole number of days");
    assert!(today > 40000.0 && today < 60000.0, "TODAY() is implausibly far out: {today}");

    let now = match e("=NOW()") {
        Value::Number(n) => n,
        other => panic!("NOW() gave {other:?}"),
    };
    assert!(now >= today && now < today + 1.0, "NOW() ({now}) is not within TODAY ({today})");
}

// Volatile cells lack precedents; every recalc must seed them
#[test]
fn an_unrelated_edit_recalculates_volatile_cells() {
    use engine::Sheet;

    let cell = |a1: &str| CellRef::parse_a1(a1).unwrap();
    let mut sheet = Sheet::new();
    sheet.set_input(cell("A1"), "1");
    sheet.set_input(cell("D4"), "=TODAY()");
    sheet.set_input(cell("E4"), "=NOW()");

    let report = sheet.set_input(cell("A1"), "2");
    assert!(report.dirty.contains(&cell("D4")), "dirty set was {:?}", report.dirty);
    assert!(report.dirty.contains(&cell("E4")), "dirty set was {:?}", report.dirty);
    assert_eq!(report.recalculated(), 3, "A1, TODAY and NOW should all run");
}

#[test]
fn the_suppressed_sides_are_really_not_evaluated() {
    check! {
        "=IF(TRUE, 1, 1/0)" => n(1.0),
        "=IF(FALSE, 1/0, 2)" => n(2.0),
        "=IFERROR(1/0, 42)" => n(42.0),
        "=IFERROR(7, 42)" => n(7.0),
        "=IFNA(#N/A, 42)" => n(42.0),
        "=IFNA(#DIV/0!, 42)" => bad(ErrorKind::Div0),
        "=IFNA(#VALUE!, 42)" => bad(ErrorKind::Value),
        "=IFERROR(#N/A, 42)" => n(42.0),
        "=IFS(FALSE, 1, TRUE, 2)" => n(2.0),
        "=IFS(TRUE, 1, 1/0, 2)" => n(1.0),
        "=IFS(FALSE, 1, FALSE, 2)" => bad(ErrorKind::NA),
        "=IFERROR(IFERROR(1/0, 2/0), \"both failed\")" => t("both failed"),
        "=SUM(IFERROR(1/0, 3), 4)" => n(7.0),
    }
}
