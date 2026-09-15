//! The sheet: sparse cell storage plus incremental recalculation.
//!
//! # Storage
//!
//! Cells live in a `HashMap<CellRef, Cell>`, so an unused cell costs nothing: a
//! sheet with three values in a million-row grid stores three entries. There is no
//! dense matrix anywhere, which is what lets the UI offer 100k+ rows.
//!
//! # Recalculation
//!
//! Editing a cell runs five steps:
//!
//! 1. **detach** — drop the edited cell's old precedents (edges and range watches);
//! 2. **store** — write the new input (a formula starts out unevaluated);
//! 3. **attach** — rebuild edges from the new program's precedents, and re-expand
//!    the ranges of any formula that watches the edited cell, since a cell gaining
//!    or losing a value changes which cells a range covers;
//! 4. **mark** — take the transitive closure of dependents (the *dirty* set);
//! 5. **schedule** — levelize the dirty set, then evaluate each level: cells in the
//!    same level are mutually independent, so a level with more than one cell is
//!    evaluated in parallel with `rayon`.
//!
//! Cells that sit on a cycle are given `#CYCLE!` instead of being evaluated, and
//! the cells downstream of them are still evaluated (they observe the error value).
//! Nothing ever recurses, so a circular reference cannot hang the engine.

use std::collections::{HashMap, HashSet, VecDeque};

use rayon::prelude::*;

use crate::addr::{Bounds, CellRef};
use crate::compile::{self, Precedent, Program};
use crate::error::{Diagnostic, ErrorKind};
use crate::eval::{evaluate, ValueSource};
use crate::graph::{levelize, DepGraph};
use crate::value::Value;

/// What a cell contains.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    Literal(Value),
    Formula(Formula),
}

impl Input {
    pub fn as_formula(&self) -> Option<&Formula> {
        match self {
            Input::Formula(f) => Some(f),
            Input::Literal(_) => None,
        }
    }
}

/// A formula cell: the text the user typed plus its compiled program.
#[derive(Clone, Debug, PartialEq)]
pub struct Formula {
    /// Source text as typed, including the leading `=`.
    pub source: String,
    /// Compiled program; `None` when the source could not be parsed or compiled.
    pub program: Option<Program>,
    /// Why compilation failed, positioned within [`Formula::source`].
    pub error: Option<Diagnostic>,
}

impl Formula {
    /// Compile formula text (which may or may not start with `=`).
    pub fn new(source: impl Into<String>) -> Formula {
        let source = source.into();
        match compile::compile_source(&source) {
            Ok(program) => Formula { source, program: Some(program), error: None },
            Err(error) => Formula { source, program: None, error: Some(error) },
        }
    }
}

/// A stored cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub input: Input,
    pub value: Value,
}

/// What a recalculation did — useful for the UI, for tests, and for profiling.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecalcReport {
    /// Cells identified as needing recomputation (transitive dependents).
    pub dirty: Vec<CellRef>,
    /// The evaluation batches actually run, in order.
    pub levels: Vec<Vec<CellRef>>,
    /// Cells found to be part of a circular reference and given `#CYCLE!`.
    pub cycles: Vec<CellRef>,
}

impl RecalcReport {
    /// How many cells were actually evaluated.
    pub fn recalculated(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    /// How many batches the work was split into.
    pub fn depth(&self) -> usize {
        self.levels.len()
    }
}

/// A sheet of sparse cells.
#[derive(Clone, Debug, Default)]
pub struct Sheet {
    cells: HashMap<CellRef, Cell>,
    graph: DepGraph,
}

impl Sheet {
    pub fn new() -> Sheet {
        Sheet::default()
    }

    // --- reading ---------------------------------------------------------

    /// The stored cell, if the cell is in use.
    pub fn cell(&self, cell: CellRef) -> Option<&Cell> {
        self.cells.get(&cell)
    }

    /// The computed value of a cell ([`Value::Empty`] when unused).
    pub fn value(&self, cell: CellRef) -> Value {
        match self.cells.get(&cell) {
            Some(cell) => cell.value.clone(),
            None => Value::Empty,
        }
    }

    /// The text a grid should show in this cell.
    pub fn display(&self, cell: CellRef) -> String {
        self.value(cell).as_text()
    }

    /// The formula source typed into the cell, if it holds a formula.
    pub fn formula_source(&self, cell: CellRef) -> Option<&str> {
        self.cells.get(&cell)?.input.as_formula().map(|f| f.source.as_str())
    }

    /// The parse/compile diagnostic for a broken formula.
    pub fn formula_error(&self, cell: CellRef) -> Option<&Diagnostic> {
        self.cells.get(&cell)?.input.as_formula()?.error.as_ref()
    }

    /// Whether the cell holds a formula that failed to compile.
    pub fn is_invalid(&self, cell: CellRef) -> bool {
        self.formula_error(cell).is_some()
    }

    /// Iterate over the cells in use.
    pub fn iter_cells(&self) -> impl Iterator<Item = (CellRef, &Cell)> {
        self.cells.iter().map(|(cell, stored)| (*cell, stored))
    }

    /// Number of cells in use.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// The smallest rectangle containing every cell in use.
    pub fn used_bounds(&self) -> Option<Bounds> {
        let mut iter = self.cells.keys().copied();
        let first = iter.next()?;
        let mut bounds = Bounds::single(first);
        for cell in iter {
            bounds.min_row = bounds.min_row.min(cell.row);
            bounds.max_row = bounds.max_row.max(cell.row);
            bounds.min_col = bounds.min_col.min(cell.col);
            bounds.max_col = bounds.max_col.max(cell.col);
        }
        Some(bounds)
    }

    /// Cells that read `cell` (directly, or through a range).
    pub fn dependents(&self, cell: CellRef) -> Vec<CellRef> {
        self.graph.dependents(cell)
    }

    /// Cells that `cell` reads, expanded to individual cells.
    pub fn precedents(&self, cell: CellRef) -> Vec<CellRef> {
        self.graph.precedents(cell)
    }

    /// Formulas that read a range covering `cell`.
    pub fn range_watchers(&self, cell: CellRef) -> Vec<CellRef> {
        self.graph.range_watchers(cell)
    }

    /// `(nodes, edges)` in the dependency graph.
    pub fn graph_size(&self) -> (usize, usize) {
        (self.graph.node_count(), self.graph.edge_count())
    }

    // --- editing ---------------------------------------------------------

    /// Set a cell from text, the way the formula bar or a keypress would.
    ///
    /// Text starting with `=` is a formula; anything else is a literal (number,
    /// boolean, error literal, or text). Empty text clears the cell.
    pub fn set_input(&mut self, cell: CellRef, text: &str) -> RecalcReport {
        let seeds = self.apply_input(cell, parse_input(text));
        self.recalculate(&seeds)
    }

    /// Store a prepared input (used by the loader, the UI, and tests).
    pub fn set_input_value(&mut self, cell: CellRef, input: Input) -> RecalcReport {
        let seeds = self.apply_input(cell, Some(input));
        self.recalculate(&seeds)
    }

    /// Set a cell to an already-computed value.
    pub fn set_value(&mut self, cell: CellRef, value: Value) -> RecalcReport {
        let input = if value.is_empty() { None } else { Some(Input::Literal(value)) };
        let seeds = self.apply_input(cell, input);
        self.recalculate(&seeds)
    }

    /// Set a cell to a formula.
    pub fn set_formula(&mut self, cell: CellRef, source: &str) -> RecalcReport {
        let seeds = self.apply_input(cell, Some(Input::Formula(Formula::new(source))));
        self.recalculate(&seeds)
    }

    /// Clear a cell.
    pub fn clear(&mut self, cell: CellRef) -> RecalcReport {
        let seeds = self.apply_input(cell, None);
        self.recalculate(&seeds)
    }

    /// Recompute every cell in the sheet (used after loading a file).
    pub fn recalculate_all(&mut self) -> RecalcReport {
        let mut seeds: Vec<CellRef> = self.cells.keys().copied().collect();
        seeds.sort_unstable();
        self.recalculate(&seeds)
    }

    /// Copy a block of cells elsewhere, adjusting relative references.
    ///
    /// This is the fill handle and copy/paste primitive: `source` is the block being
    /// copied, `target` the block being written, and the source block is tiled
    /// across the target when the target is larger (so dragging a formula down
    /// repeats it, with each copy offset by its distance from its source cell).
    pub fn fill(&mut self, source: Bounds, target: Bounds) -> RecalcReport {
        // Snapshot the source block first: an overlapping target must not read
        // cells that this same fill has already overwritten.
        let snapshot: HashMap<CellRef, Option<Input>> = source
            .iter_cells()
            .map(|cell| (cell, self.cells.get(&cell).map(|stored| stored.input.clone())))
            .collect();

        let mut seeds = Vec::new();
        for cell in target.iter_cells() {
            let row_offset = (i64::from(cell.row) - i64::from(source.min_row))
                .rem_euclid(i64::from(source.rows()));
            let col_offset = (i64::from(cell.col) - i64::from(source.min_col))
                .rem_euclid(i64::from(source.cols()));
            let origin = CellRef::new(
                source.min_row + row_offset as u32,
                source.min_col + col_offset as u32,
            );
            // A target cell that is its own source needs no work (delta 0).
            if origin == cell {
                continue;
            }
            let row_delta = i64::from(cell.row) - i64::from(origin.row);
            let col_delta = i64::from(cell.col) - i64::from(origin.col);
            let input = snapshot.get(&origin).cloned().flatten().map(|input| shift_input(input, row_delta, col_delta));
            seeds.extend(self.apply_input(cell, input));
        }
        seeds.sort_unstable();
        seeds.dedup();
        self.recalculate(&seeds)
    }

    // --- recalculation internals ----------------------------------------

    /// Write a cell and update the dependency graph, without recalculating.
    ///
    /// Returns the seeds for the following recalculation: the edited cell plus any
    /// formula whose range coverage changed because of it.
    fn apply_input(&mut self, cell: CellRef, input: Option<Input>) -> Vec<CellRef> {
        self.graph.clear_precedents(cell);

        match input {
            None => {
                self.cells.remove(&cell);
            }
            Some(input) => {
                // Formulas start unevaluated: the recalculation below computes them,
                // in the correct order with respect to their precedents.
                let value = match &input {
                    Input::Literal(value) => value.clone(),
                    Input::Formula(_) => Value::Empty,
                };
                self.cells.insert(cell, Cell { input, value });
            }
        }

        let precedents = self.precedents_of(cell);
        self.graph.set_precedents(cell, &precedents);

        if !self.cells.contains_key(&cell) {
            self.graph.remove_if_isolated(cell);
        }

        // Nothing else needs rebuilding: ranges are tracked as watches, so a cell
        // gaining or losing a value is already accounted for by the watch list the
        // dirty closure consults.
        vec![cell]
    }

    fn precedents_of(&self, cell: CellRef) -> Vec<Precedent> {
        match self.cells.get(&cell).map(|stored| &stored.input) {
            Some(Input::Formula(formula)) => {
                formula.program.as_ref().map(Program::precedents).unwrap_or_default()
            }
            _ => Vec::new(),
        }
    }

    /// Everything that must be recomputed: the seeds and all their dependents.
    ///
    /// Dependents come from direct references (graph edges) and from ranges (the
    /// watch list), which is why a cell that gains its very first value still
    /// triggers the formulas whose ranges cover it.
    fn dirty_closure(&self, seeds: &[CellRef]) -> Vec<CellRef> {
        let mut seen: HashSet<CellRef> = HashSet::new();
        let mut queue: VecDeque<CellRef> = VecDeque::new();
        for seed in seeds {
            if seen.insert(*seed) {
                queue.push_back(*seed);
            }
        }
        let mut next: Vec<CellRef> = Vec::new();
        while let Some(cell) = queue.pop_front() {
            next.clear();
            next.extend(self.graph.dependents(cell));
            next.extend(self.graph.range_watchers(cell));
            for dependent in next.drain(..) {
                if seen.insert(dependent) {
                    queue.push_back(dependent);
                }
            }
        }
        let mut out: Vec<CellRef> = seen.into_iter().collect();
        out.sort_unstable();
        out
    }

    /// Recompute the seeds and everything downstream of them.
    pub fn recalculate(&mut self, seeds: &[CellRef]) -> RecalcReport {
        let dirty = self.dirty_closure(seeds);
        if dirty.is_empty() {
            return RecalcReport::default();
        }
        let (levels, cycles) = levelize(&self.graph, &dirty);

        // Cells on a cycle are not evaluated at all: they get the cycle error, and
        // the cells downstream of them read that error as a value.
        for cell in &cycles {
            if let Some(stored) = self.cells.get_mut(cell) {
                stored.value = Value::Error(ErrorKind::Cycle);
            }
        }

        for level in &levels {
            let updates: Vec<(CellRef, Value)> = if level.len() < 2 {
                level.iter().map(|cell| (*cell, self.eval_cell(*cell))).collect()
            } else {
                // Independent cells: evaluate them in parallel. Values are collected
                // and applied afterwards, so the sheet is only read while this runs.
                level.par_iter().map(|cell| (*cell, self.eval_cell(*cell))).collect()
            };
            for (cell, value) in updates {
                if let Some(stored) = self.cells.get_mut(&cell) {
                    stored.value = value;
                }
            }
        }

        RecalcReport { dirty, levels, cycles }
    }

    /// Compute one cell's value from its precedents' *current* values.
    fn eval_cell(&self, cell: CellRef) -> Value {
        match self.cells.get(&cell) {
            None => Value::Empty,
            Some(stored) => match &stored.input {
                Input::Literal(value) => value.clone(),
                Input::Formula(formula) => match &formula.program {
                    Some(program) => evaluate(program, self, cell),
                    None => Value::Error(ErrorKind::Parse),
                },
            },
        }
    }
}

impl ValueSource for Sheet {
    fn value(&self, cell: CellRef) -> Value {
        self.cells.get(&cell).map(|stored| stored.value.clone()).unwrap_or(Value::Empty)
    }

    fn each_stored(&self, visit: &mut dyn FnMut(CellRef, &Value)) {
        for (cell, stored) in &self.cells {
            visit(*cell, &stored.value);
        }
    }
}

/// Re-apply a copied input at a new position, offsetting relative references.
fn shift_input(input: Input, row_delta: i64, col_delta: i64) -> Input {
    match input {
        // Literals copy verbatim. (Lattice does not do series detection: copying
        // `1, 2` does not produce `3, 4`.)
        Input::Literal(value) => Input::Literal(value),
        Input::Formula(formula) => {
            let Ok(expr) = crate::parser::parse(&formula.source) else {
                // Unparseable source cannot be rewritten; copy it as-is.
                return Input::Formula(formula);
            };
            let text = format!("={}", expr.shifted(row_delta, col_delta));
            Input::Formula(Formula::new(text))
        }
    }
}

/// Turn typed text into a cell input. `None` means "clear the cell".
pub fn parse_input(text: &str) -> Option<Input> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.starts_with('=') {
        Some(Input::Formula(Formula::new(text)))
    } else {
        Some(Input::Literal(parse_literal(text)))
    }
}

/// Interpret typed text that is not a formula.
///
/// `TRUE`/`FALSE` become booleans, error literals (`#N/A`) become errors, anything
/// numeric becomes a number (including `5%` and `1e3`), and everything else is text.
/// Text is never silently coerced to a number, and neither is a number silently
/// turned into text.
pub fn parse_literal(text: &str) -> Value {
    let text = text.trim();
    if text.eq_ignore_ascii_case("TRUE") {
        return Value::Bool(true);
    }
    if text.eq_ignore_ascii_case("FALSE") {
        return Value::Bool(false);
    }
    if let Some(kind) = ErrorKind::from_literal(text) {
        return Value::Error(kind);
    }
    if let Some(number) = parse_number(text) {
        return Value::Number(number);
    }
    Value::Text(text.to_string())
}

fn parse_number(text: &str) -> Option<f64> {
    if let Some(prefix) = text.strip_suffix('%') {
        let number: f64 = prefix.trim().parse().ok()?;
        return number.is_finite().then(|| number / 100.0);
    }
    let number: f64 = text.parse().ok()?;
    // `NaN`, `inf` and overflowed values are not numbers a spreadsheet can hold.
    number.is_finite().then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse_a1(a1).unwrap()
    }

    fn bounds(a1: &str, b1: &str) -> Bounds {
        Bounds::new(cell(a1), cell(b1))
    }

    fn number(sheet: &Sheet, a1: &str) -> f64 {
        match sheet.value(cell(a1)) {
            Value::Number(n) => n,
            other => panic!("{a1} was {other:?}, expected a number"),
        }
    }

    #[test]
    fn stores_only_cells_in_use() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("XFD1048576"), "2");
        assert_eq!(sheet.len(), 2);
        assert_eq!(sheet.value(cell("B2")), Value::Empty);
    }

    #[test]
    fn literals_are_typed_on_entry() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "42");
        sheet.set_input(cell("A2"), "4.5e1");
        sheet.set_input(cell("A3"), "50%");
        sheet.set_input(cell("A4"), "TRUE");
        sheet.set_input(cell("A5"), "hello");
        sheet.set_input(cell("A6"), "#N/A");
        sheet.set_input(cell("A7"), " 7 ");
        assert_eq!(sheet.value(cell("A1")), Value::Number(42.0));
        assert_eq!(sheet.value(cell("A2")), Value::Number(45.0));
        assert_eq!(sheet.value(cell("A3")), Value::Number(0.5));
        assert_eq!(sheet.value(cell("A4")), Value::Bool(true));
        assert_eq!(sheet.value(cell("A5")), Value::Text("hello".into()));
        assert_eq!(sheet.value(cell("A6")), Value::Error(ErrorKind::NA));
        assert_eq!(sheet.value(cell("A7")), Value::Number(7.0));
        // `NaN` is text, not a number.
        assert_eq!(sheet.value(cell("A8")), Value::Empty);
        sheet.set_input(cell("A8"), "NaN");
        assert_eq!(sheet.value(cell("A8")), Value::Text("NaN".into()));
    }

    #[test]
    fn formulas_evaluate_and_track_their_inputs() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "2");
        sheet.set_input(cell("A2"), "3");
        sheet.set_input(cell("B1"), "=A1*A2");
        assert_eq!(number(&sheet, "B1"), 6.0);

        // Changing a precedent updates the dependent.
        sheet.set_input(cell("A2"), "4");
        assert_eq!(number(&sheet, "B1"), 8.0);

        // Clearing a precedent does too (empty coerces to 0).
        sheet.clear(cell("A2"));
        assert_eq!(number(&sheet, "B1"), 0.0);
    }

    #[test]
    fn only_dirty_cells_are_recalculated() {
        let mut sheet = Sheet::new();
        for (i, a1) in ["A1", "A2", "A3", "A4"].iter().enumerate() {
            sheet.set_input(cell(a1), &i.to_string());
        }
        sheet.set_input(cell("B1"), "=A1*2");
        sheet.set_input(cell("B2"), "=A2*2");
        sheet.set_input(cell("C1"), "=SUM(A1:A4)");

        let report = sheet.set_input(cell("A1"), "10");
        // A1 changes: B1 and C1 depend on it; B2 does not.
        assert_eq!(report.dirty, vec![cell("A1"), cell("B1"), cell("C1")]);
        assert_eq!(report.recalculated(), 3);
        // B2 was not touched: its precedent did not change.
        assert_eq!(sheet.value(cell("B2")), Value::Number(2.0));
    }

    #[test]
    fn recalculation_follows_a_chain_in_order() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("B1"), "=A1+1");
        sheet.set_input(cell("C1"), "=B1+1");
        sheet.set_input(cell("D1"), "=C1+1");
        assert_eq!(number(&sheet, "D1"), 4.0);

        let report = sheet.set_input(cell("A1"), "10");
        assert_eq!(number(&sheet, "D1"), 13.0);
        // One cell per level: the chain cannot be parallelised, but nothing else is
        // recomputed and no cell is visited twice.
        assert_eq!(report.depth(), 4);
        assert_eq!(report.recalculated(), 4);
    }

    #[test]
    fn ranges_are_tracked_as_dependencies() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("A2"), "2");
        sheet.set_input(cell("B1"), "=SUM(A1:A10)");
        assert_eq!(number(&sheet, "B1"), 3.0);

        // A cell that was empty but inside the range now contributes.
        sheet.set_input(cell("A7"), "10");
        assert_eq!(number(&sheet, "B1"), 13.0);

        // Clearing it again removes the contribution.
        sheet.clear(cell("A7"));
        assert_eq!(number(&sheet, "B1"), 3.0);
    }

    #[test]
    fn range_dependencies_are_visible_as_watches() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("B1"), "=SUM(A1:A3)");
        // A range is a watch, not a set of edges: no graph nodes are consumed by
        // the (potentially millions of) cells it covers.
        assert_eq!(sheet.precedents(cell("B1")), Vec::new());
        assert_eq!(sheet.dependents(cell("A1")), Vec::new());
        assert_eq!(sheet.range_watchers(cell("A1")), vec![cell("B1")]);
        // Even a cell with no value at all is watched, so it marks B1 dirty the
        // moment it gains one.
        assert_eq!(sheet.range_watchers(cell("A2")), vec![cell("B1")]);
        let (nodes, edges) = sheet.graph_size();
        assert_eq!((nodes, edges), (1, 0));
    }

    #[test]
    fn self_reference_is_a_cycle_not_a_hang() {
        let mut sheet = Sheet::new();
        let report = sheet.set_input(cell("A1"), "=A1+1");
        assert_eq!(report.cycles, vec![cell("A1")]);
        assert_eq!(sheet.value(cell("A1")), Value::Error(ErrorKind::Cycle));
    }

    #[test]
    fn mutual_references_are_cycles() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "=B1+1");
        sheet.set_input(cell("B1"), "=A1+1");
        let report = sheet.recalculate_all();
        assert_eq!(report.cycles, vec![cell("A1"), cell("B1")]);
        assert_eq!(sheet.value(cell("A1")), Value::Error(ErrorKind::Cycle));
        assert_eq!(sheet.value(cell("B1")), Value::Error(ErrorKind::Cycle));

        // Breaking the cycle recovers both cells.
        sheet.set_input(cell("B1"), "5");
        assert_eq!(sheet.value(cell("A1")), Value::Number(6.0));
    }

    #[test]
    fn cells_downstream_of_a_cycle_still_evaluate() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "=B1+1");
        sheet.set_input(cell("B1"), "=A1+1");
        sheet.set_input(cell("C1"), "=A1*2");
        assert_eq!(sheet.value(cell("C1")), Value::Error(ErrorKind::Cycle));
    }

    #[test]
    fn long_chains_do_not_overflow_the_stack() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        for row in 1..2000 {
            sheet.set_input(CellRef::new(row, 0), &format!("=A{}+1", row));
        }
        assert_eq!(number(&sheet, "A2000"), 2000.0);
    }

    #[test]
    fn broken_formulas_report_a_diagnostic_and_parse_error_value() {
        let mut sheet = Sheet::new();
        let report = sheet.set_input(cell("A1"), "=1+*2");
        assert!(report.cycles.is_empty());
        assert_eq!(sheet.value(cell("A1")), Value::Error(ErrorKind::Parse));
        let diagnostic = sheet.formula_error(cell("A1")).expect("diagnostic");
        assert!(diagnostic.span.0 <= 4);
        assert!(sheet.is_invalid(cell("A1")));
        // The text is still there for the user to fix.
        assert_eq!(sheet.formula_source(cell("A1")), Some("=1+*2"));
    }

    #[test]
    fn wrong_arity_is_reported_as_invalid() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "=IF(1)");
        assert_eq!(sheet.value(cell("A1")), Value::Error(ErrorKind::Parse));
        assert!(sheet.formula_error(cell("A1")).unwrap().message.contains("IF expects"));
    }

    #[test]
    fn fill_offsets_relative_references_but_not_anchored_ones() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "10");
        sheet.set_input(cell("A2"), "20");
        sheet.set_input(cell("B1"), "=A1*$C$1+A$1+$A1");

        sheet.fill(bounds("B1", "B1"), bounds("B1", "B2"));
        assert_eq!(sheet.formula_source(cell("B2")), Some("=A2*$C$1+A$1+$A2"));
    }

    #[test]
    fn fill_tiles_a_multi_cell_source_across_a_larger_target() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("A2"), "2");
        sheet.fill(bounds("A1", "A2"), bounds("A1", "A6"));
        assert_eq!(sheet.value(cell("A3")), Value::Number(1.0));
        assert_eq!(sheet.value(cell("A4")), Value::Number(2.0));
        assert_eq!(sheet.value(cell("A5")), Value::Number(1.0));
        assert_eq!(sheet.value(cell("A6")), Value::Number(2.0));
    }

    #[test]
    fn fill_tiles_the_source_block_when_the_target_overlaps_it() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("A2"), "2");
        // Dragging A1:A2 over A2:A3 repeats the block, phase-anchored at A1: the
        // cell that is its own source is left alone, and A3 restarts the block.
        sheet.fill(bounds("A1", "A2"), bounds("A2", "A3"));
        assert_eq!(sheet.value(cell("A2")), Value::Number(2.0));
        assert_eq!(sheet.value(cell("A3")), Value::Number(1.0));
    }

    #[test]
    fn fill_recalculates_the_filled_formulas() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("A2"), "2");
        sheet.set_input(cell("A3"), "3");
        sheet.set_input(cell("B1"), "=A1*10");
        // Dragging B1 down rewrites the reference for each row and evaluates it.
        sheet.fill(bounds("B1", "B1"), bounds("B1", "B3"));
        assert_eq!(sheet.formula_source(cell("B2")), Some("=A2*10"));
        assert_eq!(sheet.value(cell("B1")), Value::Number(10.0));
        assert_eq!(sheet.value(cell("B2")), Value::Number(20.0));
        assert_eq!(sheet.value(cell("B3")), Value::Number(30.0));
    }

    #[test]
    fn fill_off_the_sheet_produces_ref_errors() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("B2"), "=A1");
        // Dragging B2 upward shifts `A1` to `A0`, which does not exist.
        sheet.fill(bounds("B2", "B2"), bounds("B1", "B2"));
        assert_eq!(sheet.formula_source(cell("B1")), Some("=#REF!"));
        assert_eq!(sheet.value(cell("B1")), Value::Error(ErrorKind::Ref));
    }

    #[test]
    fn copying_a_formula_to_a_new_place_keeps_literals_verbatim() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "hello");
        sheet.fill(bounds("A1", "A1"), bounds("B1", "B1"));
        assert_eq!(sheet.value(cell("B1")), Value::Text("hello".into()));
    }

    #[test]
    fn recalculate_all_rebuilds_every_value() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "2");
        sheet.set_input(cell("A2"), "3");
        sheet.set_input(cell("B1"), "=A1+A2");
        let report = sheet.recalculate_all();
        assert_eq!(sheet.value(cell("B1")), Value::Number(5.0));
        assert_eq!(report.recalculated(), 3);
    }

    #[test]
    fn a_cell_can_be_reused_after_being_cleared() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("A2"), "=A1+1");
        sheet.clear(cell("A1"));
        assert_eq!(number(&sheet, "A2"), 1.0);
        sheet.set_input(cell("A1"), "5");
        assert_eq!(number(&sheet, "A2"), 6.0);
    }

    #[test]
    fn the_graph_does_not_grow_without_bound_when_formulas_are_deleted() {
        let mut sheet = Sheet::new();
        for i in 0..50 {
            sheet.set_input(CellRef::new(0, i), &format!("=A{}", i + 1));
        }
        let (nodes, _) = sheet.graph_size();
        assert!(nodes > 50);
        for i in 0..50 {
            sheet.clear(CellRef::new(0, i));
        }
        // Only the referenced-but-empty column A cells may remain.
        let (nodes_after, edges_after) = sheet.graph_size();
        assert_eq!(edges_after, 0, "edges should be gone");
        assert!(nodes_after <= 51, "nodes should be pruned, got {nodes_after}");
    }

    #[test]
    fn used_bounds_describes_the_occupied_rectangle() {
        let mut sheet = Sheet::new();
        assert_eq!(sheet.used_bounds(), None);
        sheet.set_input(cell("B2"), "1");
        sheet.set_input(cell("D9"), "1");
        assert_eq!(sheet.used_bounds(), Some(Bounds::new(cell("B2"), cell("D9"))));
    }

    #[test]
    fn helpers_expose_range_and_literal_parsing() {
        assert_eq!(parse_literal("12"), Value::Number(12.0));
        assert_eq!(parse_input(""), None);
        assert!(parse_input("=1+1").unwrap().as_formula().is_some());
        assert_eq!(parse_literal("#N/A"), Value::Error(ErrorKind::NA));
    }
}
