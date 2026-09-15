//! A small recalculation benchmark.
//!
//! Run with `cargo run --release -p engine --example recalc_bench`.
//!
//! It builds a tall column of numbers, aggregates it, and then times an edit deep
//! in the column — the case that matters for interactive use, where only the edited
//! cell and the cells that read it should be recomputed.

use std::time::Instant;

use engine::{CellRef, Sheet};

fn main() {
    let rows: u32 = std::env::args()
        .nth(1)
        .and_then(|arg| arg.parse().ok())
        .unwrap_or(100_000);

    let mut sheet = Sheet::new();
    let start = Instant::now();
    for row in 0..rows {
        sheet.set_input(CellRef::new(row, 0), "1");
    }
    println!("fill {rows} cells: {:?}", start.elapsed());

    let start = Instant::now();
    sheet.set_input(CellRef::new(rows, 0), &format!("=SUM(A1:A{rows})"));
    println!("add whole-column aggregate: {:?}", start.elapsed());

    for round in 0..3 {
        let target = CellRef::new(rows / 2 + round, 0);
        let start = Instant::now();
        let report = sheet.set_input(target, &format!("{}", round + 2));
        println!(
            "edit {} (round {round}): {:?} — dirty {}, levels {}, recomputed {}",
            target.a1(),
            start.elapsed(),
            report.dirty.len(),
            report.depth(),
            report.recalculated()
        );
    }

    let start = Instant::now();
    let report = sheet.recalculate_all();
    println!(
        "full recalculation: {:?} — recomputed {} in {} levels",
        start.elapsed(),
        report.recalculated(),
        report.depth()
    );

    let start = Instant::now();
    let json = engine::io::to_string(&sheet, "bench").unwrap();
    println!("serialise: {:?} ({} bytes)", start.elapsed(), json.len());
}
