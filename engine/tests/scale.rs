
use std::time::{Duration, Instant};

use engine::{CellRef, Sheet, Value};

fn cell(a1: &str) -> CellRef {
    CellRef::parse_a1(a1).unwrap()
}

fn number(sheet: &Sheet, cell_ref: CellRef) -> f64 {
    match sheet.value(cell_ref) {
        Value::Number(n) => n,
        other => panic!("{} was {other:?}", cell_ref.a1()),
    }
}

#[test]
fn editing_one_among_a_hundred_thousand_rows_is_cheap() {
    let mut sheet = Sheet::new();
    let start = Instant::now();
    for row in 0..100_000u32 {
        sheet.set_input(CellRef::new(row, 0), "1");
    }
    let build = start.elapsed();

    // the total watches the whole column as one range
    let start = Instant::now();
    sheet.set_input(CellRef::new(100_000, 0), "=SUM(A1:A100000)");
    let aggregate = start.elapsed();
    assert_eq!(number(&sheet, CellRef::new(100_000, 0)), 100_000.0);

    // editing one cell dirties only it and the aggregate
    let start = Instant::now();
    let report = sheet.set_input(CellRef::new(50_000, 0), "10");
    let edit = start.elapsed();

    assert_eq!(report.dirty.len(), 2, "dirty set was {:?}", report.dirty);
    assert_eq!(report.recalculated(), 2);
    assert_eq!(number(&sheet, CellRef::new(100_000, 0)), 100_009.0);

    assert!(build < Duration::from_secs(20), "building took {build:?}");
    assert!(aggregate < Duration::from_secs(10), "aggregate took {aggregate:?}");
    assert!(edit < Duration::from_secs(1), "editing took {edit:?}");
}

// chain recalc is linear, not recursive per cell
#[test]
fn a_deep_chain_of_dependents_is_linear() {
    let mut sheet = Sheet::new();
    sheet.set_input(CellRef::new(0, 0), "1");
    for row in 1..20_000u32 {
        sheet.set_input(CellRef::new(row, 0), &format!("=A{}+1", row));
    }

    let start = Instant::now();
    let report = sheet.set_input(CellRef::new(0, 0), "2");
    let elapsed = start.elapsed();

    assert_eq!(report.recalculated(), 20_000);
    assert_eq!(number(&sheet, CellRef::new(19_999, 0)), 20_001.0);
    assert!(elapsed < Duration::from_secs(10), "chain took {elapsed:?}");
}

// wide-and-shallow: all dependents of one cell share a level
#[test]
fn independent_cells_share_a_single_level() {
    let mut sheet = Sheet::new();
    sheet.set_input(cell("A1"), "3");
    for col in 1..1_000u32 {
        sheet.set_input(CellRef::new(0, col), "=A1*2");
    }

    let report = sheet.set_input(cell("A1"), "4");
    assert_eq!(report.depth(), 2, "levels: {:?}", report.levels.len());
    assert_eq!(report.levels[1].len(), 999);
    assert_eq!(number(&sheet, CellRef::new(0, 500)), 8.0);
}

// whole-column ranges stay watches, not a million edges
#[test]
fn whole_column_ranges_do_not_explode() {
    let mut sheet = Sheet::new();
    for row in 0..5u32 {
        sheet.set_input(CellRef::new(row, 0), "1");
    }
    let start = Instant::now();
    sheet.set_input(cell("C1"), "=SUM(A1:A1048576)");
    let elapsed = start.elapsed();

    assert_eq!(number(&sheet, cell("C1")), 5.0);
    let (nodes, edges) = sheet.graph_size();
    assert!(nodes < 20, "graph grew to {nodes} nodes");
    assert!(edges < 20, "graph grew to {edges} edges");
    assert!(elapsed < Duration::from_secs(1), "range setup took {elapsed:?}");
}

#[test]
fn large_sheets_round_trip() {
    let mut sheet = Sheet::new();
    for row in 0..50_000u32 {
        sheet.set_input(CellRef::new(row, 0), &format!("{}", row % 97));
    }
    sheet.set_input(CellRef::new(50_000, 0), "=AVERAGE(A1:A50000)");

    let start = Instant::now();
    let json = engine::io::to_string(&sheet, "big").unwrap();
    let loaded = engine::io::from_str(&json).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(loaded.len(), sheet.len());
    assert_eq!(loaded.value(CellRef::new(50_000, 0)), sheet.value(CellRef::new(50_000, 0)));
    assert!(elapsed < Duration::from_secs(30), "round trip took {elapsed:?}");
}
