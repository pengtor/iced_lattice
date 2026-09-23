use super::*;

pub(super) fn dispatch(
    func: FuncId,
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
) -> Option<Value> {
    Some(match func {
        FuncId::Len => match text_arg(args, 0, source, host) {
            Ok(text) => Value::Number(text.chars().count() as f64),
            Err(kind) => Value::Error(kind),
        },
        FuncId::Upper => unary_text(args, source, host, |t| t.to_uppercase()),
        FuncId::Lower => unary_text(args, source, host, |t| t.to_lowercase()),
        FuncId::Trim => unary_text(args, source, host, trim_excel),
        FuncId::Left => left(args, source, host),
        FuncId::Right => right(args, source, host),
        FuncId::Mid => mid(args, source, host),
        FuncId::Find => find(args, source, host, false),
        FuncId::Search => find(args, source, host, true),
        FuncId::Substitute => substitute(args, source, host),
        FuncId::Replace => replace(args, source, host),
        FuncId::Text => text_format(args, source, host),
        FuncId::ValueFn => value_fn(args, source, host),
        _ => return None,
    })
}

fn unary_text(
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
    transform: impl FnOnce(&str) -> String,
) -> Value {
    match text_arg(args, 0, source, host) {
        Ok(text) => Value::Text(transform(&text)),
        Err(kind) => Value::Error(kind),
    }
}

// Excel TRIM: only ASCII spaces collapse; tabs survive
fn trim_excel(text: &str) -> String {
    text.split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn left(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let text = match text_arg(args, 0, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let count = match opt_count_arg(args, 1, source, host, 1) {
        Ok(count) => count,
        Err(kind) => return Value::Error(kind),
    };
    if count < 0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = text.chars().collect();
    let take = (count as usize).min(chars.len());
    let taken: String = chars[..take].iter().collect();
    Value::Text(taken)
}

fn right(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let text = match text_arg(args, 0, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let count = match opt_count_arg(args, 1, source, host, 1) {
        Ok(count) => count,
        Err(kind) => return Value::Error(kind),
    };
    if count < 0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = text.chars().collect();
    let skip = chars.len().saturating_sub(count as usize);
    let taken: String = chars[skip..].iter().collect();
    Value::Text(taken)
}

fn mid(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let text = match text_arg(args, 0, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let start = match count_arg(args, 1, source, host) {
        Ok(start) => start,
        Err(kind) => return Value::Error(kind),
    };
    let count = match count_arg(args, 2, source, host) {
        Ok(count) => count,
        Err(kind) => return Value::Error(kind),
    };
    if start < 1 || count < 0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = text.chars().collect();
    let skip = (start - 1) as usize;
    let taken: String = chars.iter().skip(skip).take(count as usize).collect();
    Value::Text(taken)
}

fn find(
    args: &[Operand],
    source: &dyn ValueSource,
    host: CellRef,
    case_insensitive: bool,
) -> Value {
    let needle = match text_arg(args, 0, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let within = match text_arg(args, 1, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let start = match opt_count_arg(args, 2, source, host, 1) {
        Ok(start) => start,
        Err(kind) => return Value::Error(kind),
    };
    let within_chars: Vec<char> = within.chars().collect();
    if start < 1 || start as i64 > within_chars.len() as i64 {
        return Value::Error(ErrorKind::Value);
    }
    let start0 = (start - 1) as usize;

    let found = if case_insensitive {
        wildcard_find(&needle, &within, start0)
    } else {
        let needle_chars: Vec<char> = needle.chars().collect();
        find_chars(&within_chars, &needle_chars, start0)
    };

    match found {
        Some(index) => Value::Number((index + 1) as f64),
        None => Value::Error(ErrorKind::Value),
    }
}

// Empty needle matches at `from`, so FIND("", "abc") is 1
fn find_chars(haystack: &[char], needle: &[char], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from.min(haystack.len()));
    }
    if needle.len() > haystack.len() || from > haystack.len() - needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len())
        .find(|&i| &haystack[i..i + needle.len()] == needle)
}

fn substitute(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let text = match text_arg(args, 0, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let old = match text_arg(args, 1, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let new = match text_arg(args, 2, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let instance = if args.get(3).is_none() {
        None
    } else {
        match count_arg(args, 3, source, host) {
            Ok(value) => Some(value),
            Err(kind) => return Value::Error(kind),
        }
    };
    if let Some(n) = instance {
        if n < 1 {
            return Value::Error(ErrorKind::Value);
        }
    }
    // Empty `old` would expand forever, so return the text unchanged
    if old.is_empty() {
        return Value::Text(text);
    }
    Value::Text(substitute_instances(&text, &old, &new, instance))
}

fn substitute_instances(text: &str, old: &str, new: &str, instance: Option<i64>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut seen = 0i64;
    while let Some(index) = rest.find(old) {
        out.push_str(&rest[..index]);
        seen += 1;
        if instance.is_none() || instance == Some(seen) {
            out.push_str(new);
        } else {
            out.push_str(old);
        }
        rest = &rest[index + old.len()..];
    }
    out.push_str(rest);
    out
}

fn replace(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let text = match text_arg(args, 0, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    let start = match count_arg(args, 1, source, host) {
        Ok(start) => start,
        Err(kind) => return Value::Error(kind),
    };
    let count = match count_arg(args, 2, source, host) {
        Ok(count) => count,
        Err(kind) => return Value::Error(kind),
    };
    let new = match text_arg(args, 3, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    if start < 1 || count < 0 {
        return Value::Error(ErrorKind::Value);
    }
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len() as u64;
    let from = (start as u64 - 1).min(len) as usize;
    let to = (start as u64 - 1).saturating_add(count as u64).min(len) as usize;

    let mut out = String::new();
    out.extend(chars[..from].iter());
    out.push_str(&new);
    out.extend(chars[to..].iter());
    Value::Text(out)
}

#[derive(Clone, Copy)]
struct FormatItem {
    ch: char,
    quoted: bool,
}

fn is_placeholder(c: char) -> bool {
    matches!(c, '0' | '#' | '.' | ',' | '%')
}

fn text_format(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    let number = match number_arg(args, 0, source, host) {
        Ok(number) => number,
        Err(kind) => return Value::Error(kind),
    };
    let format = match text_arg(args, 1, source, host) {
        Ok(text) => text,
        Err(kind) => return Value::Error(kind),
    };
    Value::Text(format_number(number, &format))
}

fn tokenize_format(format: &str) -> Vec<FormatItem> {
    let mut items = Vec::new();
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' {
            items.push(FormatItem { ch: c, quoted: false });
            continue;
        }
        loop {
            match chars.next() {
                Some('"') => {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        items.push(FormatItem { ch: '"', quoted: true });
                    } else {
                        break;
                    }
                }
                Some(other) => items.push(FormatItem { ch: other, quoted: true }),
                None => break,
            }
        }
    }
    items
}

fn placeholder_section(items: &[FormatItem]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < items.len() {
        if items[i].quoted || !is_placeholder(items[i].ch) {
            i += 1;
            continue;
        }
        let start = i;
        let mut has_digit = false;
        while i < items.len() && !items[i].quoted && is_placeholder(items[i].ch) {
            if matches!(items[i].ch, '0' | '#') {
                has_digit = true;
            }
            i += 1;
        }
        if has_digit {
            return Some((start, i));
        }
    }
    None
}

fn format_number(value: f64, format: &str) -> String {
    let items = tokenize_format(format);
    let (section_start, section_end) = match placeholder_section(&items) {
        Some(section) => section,
        None => return items.iter().map(|item| item.ch).collect(),
    };

    let prefix: String = items[..section_start].iter().map(|item| item.ch).collect();
    let suffix: String = items[section_end..].iter().map(|item| item.ch).collect();

    let mut min_int_digits = 0usize;
    let mut frac_digits = 0usize;
    let mut grouping = false;
    let mut percent = false;
    let mut after_point = false;
    for item in &items[section_start..section_end] {
        match item.ch {
            '0' => {
                if after_point {
                    frac_digits += 1;
                } else {
                    min_int_digits += 1;
                }
            }
            '#' => {
                if after_point {
                    frac_digits += 1;
                }
            }
            '.' => after_point = true,
            ',' => grouping = true,
            '%' => percent = true,
            _ => {}
        }
    }

    let scaled = if percent { value * 100.0 } else { value };
    let rendered = fixed_decimal(scaled, frac_digits);
    let (sign, rest) = match rendered.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", rendered.as_str()),
    };
    let (int_part, frac_part) = match rest.split_once('.') {
        Some((int_part, frac_part)) => (int_part, frac_part),
        None => (rest, ""),
    };

    let mut digits = "0".repeat(min_int_digits.saturating_sub(int_part.len()));
    digits.push_str(int_part);
    let mut body = if frac_digits > 0 {
        format!("{sign}{digits}.{frac_part}")
    } else {
        format!("{sign}{digits}")
    };
    if grouping {
        body = group_thousands(&body);
    }

    let mut out = prefix;
    out.push_str(&body);
    if percent {
        out.push('%');
    }
    out.push_str(&suffix);
    out
}

fn value_fn(args: &[Operand], source: &dyn ValueSource, host: CellRef) -> Value {
    match value_arg(args, 0, source, host) {
        Value::Number(n) => Value::Number(n),
        Value::Bool(b) => Value::Number(if b { 1.0 } else { 0.0 }),
        Value::Error(kind) => Value::Error(kind),
        Value::Text(text) => match parse_number(&text) {
            Some(n) => Value::finite_number(n),
            None => Value::Error(ErrorKind::Value),
        },
        // Blank cells read as ""; VALUE("") is an error too
        Value::Empty => Value::Error(ErrorKind::Value),
    }
}

fn parse_number(raw: &str) -> Option<f64> {
    let mut text = raw.trim();
    if text.is_empty() {
        return None;
    }

    let mut negative = false;
    if text.starts_with('(') && text.ends_with(')') && text.len() >= 2 {
        negative = true;
        text = text[1..text.len() - 1].trim();
    }

    let mut sign_seen = false;
    let mut currency_seen = false;
    loop {
        if !sign_seen && (text.starts_with('-') || text.starts_with('+')) {
            sign_seen = true;
            if text.starts_with('-') {
                negative = !negative;
            }
            text = &text[1..];
        } else if !currency_seen && text.starts_with('$') {
            currency_seen = true;
            text = &text[1..];
        } else {
            break;
        }
        text = text.trim_start();
    }

    let mut percent = false;
    if let Some(rest) = text.strip_suffix('%') {
        percent = true;
        text = rest.trim_end();
    }
    if text.is_empty() {
        return None;
    }

    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(index) => {
            let (mantissa, exponent) = text.split_at(index);
            (mantissa, parse_exponent(&exponent[1..])?)
        }
        None => (text, 0i32),
    };

    if mantissa.is_empty() {
        return None;
    }
    let (int_str, frac_str) = match mantissa.split_once('.') {
        Some((int_str, frac_str)) => {
            if frac_str.contains('.') {
                return None;
            }
            (int_str, frac_str)
        }
        None => (mantissa, ""),
    };
    if int_str.is_empty() && frac_str.is_empty() {
        return None;
    }
    if !valid_grouped_digits(int_str) || !frac_str.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    // Let f64 parse the rebuilt string so exponents stay exact
    let int_clean: String = int_str.chars().filter(|c| *c != ',').collect();
    let mut plain = if int_clean.is_empty() { "0".to_string() } else { int_clean };
    if !frac_str.is_empty() {
        plain.push('.');
        plain.push_str(frac_str);
    }
    let value: f64 = format!("{plain}e{exponent}").parse().ok()?;
    if !value.is_finite() {
        return None;
    }

    let value = if percent { value / 100.0 } else { value };
    Some(if negative { -value } else { value })
}

// `,` is valid only as correctly placed thousands separators
fn valid_grouped_digits(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    let all_digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !text.contains(',') {
        return all_digits(text);
    }
    let groups: Vec<&str> = text.split(',').collect();
    if !(1..=3).contains(&groups[0].len()) || !all_digits(groups[0]) {
        return false;
    }
    groups[1..].iter().all(|group| group.len() == 3 && all_digits(group))
}

fn parse_exponent(text: &str) -> Option<i32> {
    let (negative, digits) = if let Some(rest) = text.strip_prefix('-') {
        (true, rest)
    } else if let Some(rest) = text.strip_prefix('+') {
        (false, rest)
    } else {
        (false, text)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let magnitude: i32 = digits.parse().ok()?;
    Some(if negative { -magnitude } else { magnitude })
}

#[cfg(test)]
mod tests {
    use crate::functions::test_support::{assert_eq_value, assert_error, eval_at};
    use crate::{ErrorKind, Value};

    fn text(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn len_counts_characters_not_bytes() {
        assert_eq_value("=LEN(\"héllo\")", Value::Number(5.0));
        assert_eq_value("=LEN(\"\")", Value::Number(0.0));
        assert_eq_value("=LEN(1.5)", Value::Number(3.0));
        assert_eq_value("=LEN(TRUE)", Value::Number(4.0));
        assert_error("=LEN(#N/A)", ErrorKind::NA);
        let cells = [("A1", text("héllo"))];
        assert_eq!(eval_at("=LEN(A1)", &cells, "B1"), Value::Number(5.0));
    }

    #[test]
    fn upper_and_lower_change_case_and_propagate_errors() {
        assert_eq_value("=UPPER(\"héllo\")", text("HÉLLO"));
        assert_eq_value("=UPPER(\"abc\")", text("ABC"));
        assert_eq_value("=LOWER(\"HÉLLO\")", text("héllo"));
        assert_eq_value("=LOWER(\"MiXeD 123\")", text("mixed 123"));
        assert_eq_value("=UPPER(TRUE)", text("TRUE"));
        assert_eq_value("=UPPER(\"\")", text(""));
        assert_error("=UPPER(#VALUE!)", ErrorKind::Value);
    }

    #[test]
    fn trim_strips_the_edges_and_collapses_internal_spaces() {
        assert_eq_value("=TRIM(\"  a   b  \")", text("a b"));
        assert_eq_value("=TRIM(\"a\")", text("a"));
        assert_eq_value("=TRIM(\"   \")", text(""));
        assert_eq_value("=TRIM(\"\")", text(""));
        assert_error("=TRIM(#REF!)", ErrorKind::Ref);
        let cells = [("A1", text("a\tb"))];
        assert_eq!(eval_at("=TRIM(A1)", &cells, "B1"), text("a\tb"));
    }

    #[test]
    fn left_and_right_take_from_the_ends() {
        assert_eq_value("=LEFT(\"abc\")", text("a"));
        assert_eq_value("=LEFT(\"abc\", 2)", text("ab"));
        assert_eq_value("=LEFT(\"abc\", 10)", text("abc"));
        assert_eq_value("=LEFT(\"abc\", 0)", text(""));
        assert_eq_value("=LEFT(\"abc\", 1.9)", text("a"));
        assert_eq_value("=LEFT(\"héllo\", 2)", text("hé"));

        assert_eq_value("=RIGHT(\"abc\")", text("c"));
        assert_eq_value("=RIGHT(\"abc\", 2)", text("bc"));
        assert_eq_value("=RIGHT(\"héllo\", 2)", text("lo"));
        assert_eq_value("=RIGHT(\"abc\", 10)", text("abc"));
        assert_eq_value("=RIGHT(\"abc\", 0)", text(""));

        assert_error("=LEFT(\"abc\", -1)", ErrorKind::Value);
        assert_error("=RIGHT(\"abc\", -1)", ErrorKind::Value);
        assert_error("=LEFT(\"abc\", \"1\")", ErrorKind::Value);
    }

    #[test]
    fn mid_is_one_based_and_counts_characters() {
        assert_eq_value("=MID(\"héllo\", 2, 1)", text("é"));
        assert_eq_value("=MID(\"abc\", 1, 2)", text("ab"));
        assert_eq_value("=MID(\"abc\", 2, 10)", text("bc"));
        assert_eq_value("=MID(\"abc\", 1, 0)", text(""));
        assert_eq_value("=MID(\"abc\", 5, 2)", text(""));
        assert_error("=MID(\"abc\", 0, 1)", ErrorKind::Value);
        assert_error("=MID(\"abc\", 1, -1)", ErrorKind::Value);
        assert_error("=MID(#N/A, 1, 1)", ErrorKind::NA);
    }

    #[test]
    fn find_is_case_sensitive_while_search_is_not() {
        assert_eq_value("=FIND(\"b\", \"abc\")", Value::Number(2.0));
        assert_eq_value("=FIND(\"c\", \"abc\", 3)", Value::Number(3.0));
        assert_eq_value("=FIND(\"\", \"abc\")", Value::Number(1.0));
        assert_eq_value("=FIND(\"é\", \"héllo\")", Value::Number(2.0));
        assert_error("=FIND(\"A\", \"abc\")", ErrorKind::Value);
        assert_eq_value("=SEARCH(\"A\", \"abc\")", Value::Number(1.0));
        assert_error("=FIND(\"z\", \"abc\")", ErrorKind::Value);
        assert_error("=FIND(\"a\", \"abc\", 0)", ErrorKind::Value);
        assert_error("=FIND(\"a\", \"abc\", 5)", ErrorKind::Value);
    }

    #[test]
    fn search_understands_wildcards() {
        assert_eq_value("=SEARCH(\"B\", \"abc\")", Value::Number(2.0));
        assert_eq_value("=SEARCH(\"b*\", \"abc\")", Value::Number(2.0));
        assert_eq_value("=SEARCH(\"*bc\", \"abc\")", Value::Number(1.0));
        assert_eq_value("=SEARCH(\"a?c\", \"abc\")", Value::Number(1.0));
        assert_eq_value("=SEARCH(\"c\", \"abc\", 3)", Value::Number(3.0));
        assert_eq_value("=SEARCH(\"\", \"abc\")", Value::Number(1.0));
        assert_error("=SEARCH(\"z\", \"abc\")", ErrorKind::Value);
        assert_error("=SEARCH(\"a\", \"abc\", 9)", ErrorKind::Value);
    }

    #[test]
    fn substitute_replaces_all_or_a_single_instance() {
        assert_eq_value("=SUBSTITUTE(\"a-b-c\", \"-\", \"+\")", text("a+b+c"));
        assert_eq_value("=SUBSTITUTE(\"a-b-c\", \"-\", \"+\", 2)", text("a-b+c"));
        assert_eq_value("=SUBSTITUTE(\"a-b-c\", \"-\", \"+\", 9)", text("a-b-c"));
        assert_eq_value("=SUBSTITUTE(\"abc\", \"\", \"X\")", text("abc"));
        assert_eq_value("=SUBSTITUTE(\"abc\", \"b\", \"\")", text("ac"));
        assert_error("=SUBSTITUTE(\"a-b\", \"-\", \"+\", 0)", ErrorKind::Value);
        assert_error("=SUBSTITUTE(\"a-b\", \"-\", \"+\", -1)", ErrorKind::Value);
    }

    #[test]
    fn replace_swaps_a_character_range() {
        assert_eq_value("=REPLACE(\"abcdef\", 2, 3, \"X\")", text("aXef"));
        assert_eq_value("=REPLACE(\"abc\", 1, 0, \"X\")", text("Xabc"));
        assert_eq_value("=REPLACE(\"abc\", 10, 2, \"X\")", text("abcX"));
        assert_eq_value("=REPLACE(\"héllo\", 2, 1, \"a\")", text("hallo"));
        assert_error("=REPLACE(\"abc\", 0, 1, \"X\")", ErrorKind::Value);
        assert_error("=REPLACE(\"abc\", 1, -1, \"X\")", ErrorKind::Value);
    }

    #[test]
    fn text_formats_numbers_over_a_documented_subset() {
        assert_eq_value("=TEXT(1234.5, \"#,##0.00\")", text("1,234.50"));
        assert_eq_value("=TEXT(1234.5, \"#,##0\")", text("1,235"));
        assert_eq_value("=TEXT(0.125, \"0.0%\")", text("12.5%"));
        assert_eq_value("=TEXT(7, \"000\")", text("007"));
        assert_eq_value("=TEXT(2.675, \"0.00\")", text("2.68"));
        assert_eq_value("=TEXT(2.5, \"0\")", text("3"));
        assert_eq_value("=TEXT(-1234.5, \"#,##0.00\")", text("-1,234.50"));
        assert_eq_value("=TEXT(12, \"#,##0\"\" kg\"\"\")", text("12 kg"));
        assert_eq_value("=TEXT(12, \"hello\")", text("hello"));
        assert_error("=TEXT(\"x\", \"0\")", ErrorKind::Value);
        assert_error("=TEXT(#N/A, \"0\")", ErrorKind::NA);
    }

    #[test]
    fn value_parses_numbers_and_rejects_anything_else() {
        assert_eq_value("=VALUE(\"$1,234.50\")", Value::Number(1234.5));
        assert_eq_value("=VALUE(\"(1.5)\")", Value::Number(-1.5));
        assert_eq_value("=VALUE(\" 1.5e3 \")", Value::Number(1500.0));
        assert_eq_value("=VALUE(\"-42\")", Value::Number(-42.0));
        assert_eq_value("=VALUE(\"+3.5\")", Value::Number(3.5));
        assert_eq_value("=VALUE(\"50%\")", Value::Number(0.5));
        assert_eq_value("=VALUE(\".5\")", Value::Number(0.5));
        assert_eq_value("=VALUE(2.5)", Value::Number(2.5));
        assert_eq_value("=VALUE(TRUE)", Value::Number(1.0));
        assert_error("=VALUE(#N/A)", ErrorKind::NA);
        for bad in ["", "abc", "1.2.3", "$", "1,2", "1e"] {
            assert_error(&format!("=VALUE(\"{bad}\")"), ErrorKind::Value);
        }
    }
}
