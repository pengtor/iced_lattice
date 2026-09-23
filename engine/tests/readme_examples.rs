
use std::collections::HashMap;
use std::path::PathBuf;

use engine::{compile_source, evaluate, CellRef, Value, ValueSource};

fn sample_sheet(readme: &str) -> HashMap<CellRef, Value> {
    let mut inside = false;
    let mut sheet = HashMap::new();
    for line in readme.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            if line.contains("lattice-sample") {
                inside = true;
            } else if inside {
                break;
            }
            continue;
        }
        if !inside || line.is_empty() {
            continue;
        }
        let (cell, value) = line
            .split_once(char::is_whitespace)
            .unwrap_or_else(|| panic!("sample data line `{line}` needs a value"));
        let cell = CellRef::parse_a1(cell.trim())
            .unwrap_or_else(|| panic!("sample data line `{line}` does not start with a cell"));
        let value = value.trim();
        let value = match value.parse::<f64>() {
            Ok(n) => Value::Number(n),
            Err(_) => Value::Text(value.to_string()),
        };
        assert!(sheet.insert(cell, value).is_none(), "sample data names {cell} twice");
    }
    assert!(!sheet.is_empty(), "the README has no `lattice-sample` block");
    sheet
}

fn example(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim().strip_prefix("`=")?;
    let (formula, rest) = rest.split_once('`')?;
    // The README writes an example as `` `=formula` -> `expected` ``. Older
    // revisions used a `→` in the same slot; accept both so a line can never
    // silently stop being checked just because the arrow changed shape.
    let rest = rest.trim_start();
    let rest = rest.strip_prefix("->").or_else(|| rest.strip_prefix('→'))?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('`')?;
    let (expected, _) = rest.split_once('`')?;
    Some((formula, expected))
}

#[test]
fn every_readme_example_evaluates_to_what_it_says() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../README.md");
    let readme = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let source = sample_sheet(&readme);
    // Z100 sits below the table; no formula reads itself
    let host = CellRef::parse_a1("Z100").unwrap();

    let mut failures = Vec::new();
    let mut checked = 0;

    for (number, line) in readme.lines().enumerate() {
        let Some((formula, expected)) = example(line) else {
            continue;
        };
        checked += 1;
        let full = format!("={formula}");
        let actual = match compile_source(&full) {
            Ok(program) => evaluate(&program, &source, host),
            Err(error) => {
                failures.push(format!("line {}: {full} does not compile: {error}", number + 1));
                continue;
            }
        };
        let actual = actual.as_text();
        if actual != expected {
            failures.push(format!(
                "line {}: {full} — documented `{expected}`, actually `{actual}`",
                number + 1
            ));
        }
    }

    assert!(checked > 100, "only {checked} examples found; the README format must have changed");
    assert!(failures.is_empty(), "the README disagrees with the engine:\n{}", failures.join("\n"));
}

#[test]
fn the_sample_sheet_is_documented_for_a_reader() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../README.md");
    let readme = std::fs::read_to_string(&path).unwrap();
    let sheet = sample_sheet(&readme);
    for cell in ["A1", "A4", "B1", "B4", "C1", "C4", "D1", "D4"] {
        let cell = CellRef::parse_a1(cell).unwrap();
        assert!(sheet.contains_key(&cell), "the sample sheet does not cover {cell}");
    }
    for word in ["region", "fruit", "qty", "price"] {
        assert!(readme.contains(word), "the sample data does not explain `{word}`");
    }
}

#[test]
fn the_parser_itself_is_not_vacuous() {
    assert_eq!(example("`=ABS(-3)` -> `3` — drops the sign"), Some(("ABS(-3)", "3")));
    assert_eq!(example("  `=SUM(1, 2)` -> `3`"), Some(("SUM(1, 2)", "3")));
    assert_eq!(example("`=ABS(-3)` → `3`"), Some(("ABS(-3)", "3")));
    assert_eq!(example("`ABS(number)` — the magnitude"), None);
    assert_eq!(example("prose mentioning `=A1+1` inline"), None);
    let _ = std::any::type_name::<dyn ValueSource>();
}
