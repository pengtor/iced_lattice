
use std::collections::{HashMap, HashSet, VecDeque};

use rayon::prelude::*;

use crate::addr::{Bounds, CellRef};
use crate::compile::{self, Precedent, Program};
use crate::error::{Diagnostic, ErrorKind};
use crate::eval::{evaluate, ValueSource};
use crate::graph::{levelize, DepGraph};
use crate::value::Value;

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

#[derive(Clone, Debug, PartialEq)]
pub struct Formula {
    pub source: String,
    pub program: Option<Program>,
    pub error: Option<Diagnostic>,
}

impl Formula {
    pub fn new(source: impl Into<String>) -> Formula {
        let source = source.into();
        match compile::compile_source(&source) {
            Ok(program) => Formula { source, program: Some(program), error: None },
            Err(error) => Formula { source, program: None, error: Some(error) },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cell {
    pub input: Input,
    pub value: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecalcReport {
    pub dirty: Vec<CellRef>,
    pub levels: Vec<Vec<CellRef>>,
    pub cycles: Vec<CellRef>,
}

impl RecalcReport {
    pub fn recalculated(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    pub fn depth(&self) -> usize {
        self.levels.len()
    }
}

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    cells: HashMap<CellRef, Cell>,
    graph: DepGraph,
    volatile_cells: HashSet<CellRef>,
}

impl Sheet {
    pub fn new() -> Sheet {
        Sheet::default()
    }


    pub fn cell(&self, cell: CellRef) -> Option<&Cell> {
        self.cells.get(&cell)
    }

    pub fn value(&self, cell: CellRef) -> Value {
        match self.cells.get(&cell) {
            Some(cell) => cell.value.clone(),
            None => Value::Empty,
        }
    }

    pub fn display(&self, cell: CellRef) -> String {
        self.value(cell).as_text()
    }

    pub fn formula_source(&self, cell: CellRef) -> Option<&str> {
        self.cells.get(&cell)?.input.as_formula().map(|f| f.source.as_str())
    }

    pub fn formula_error(&self, cell: CellRef) -> Option<&Diagnostic> {
        self.cells.get(&cell)?.input.as_formula()?.error.as_ref()
    }

    pub fn is_invalid(&self, cell: CellRef) -> bool {
        self.formula_error(cell).is_some()
    }

    pub fn iter_cells(&self) -> impl Iterator<Item = (CellRef, &Cell)> {
        self.cells.iter().map(|(cell, stored)| (*cell, stored))
    }

    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

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

    pub fn dependents(&self, cell: CellRef) -> Vec<CellRef> {
        self.graph.dependents(cell)
    }

    pub fn precedents(&self, cell: CellRef) -> Vec<CellRef> {
        self.graph.precedents(cell)
    }

    pub fn range_watchers(&self, cell: CellRef) -> Vec<CellRef> {
        self.graph.range_watchers(cell)
    }

    pub fn graph_size(&self) -> (usize, usize) {
        (self.graph.node_count(), self.graph.edge_count())
    }


    pub fn set_input(&mut self, cell: CellRef, text: &str) -> RecalcReport {
        let seeds = self.apply_input(cell, parse_input(text));
        self.recalculate(&seeds)
    }

    pub fn set_input_value(&mut self, cell: CellRef, input: Input) -> RecalcReport {
        let seeds = self.apply_input(cell, Some(input));
        self.recalculate(&seeds)
    }

    pub fn set_value(&mut self, cell: CellRef, value: Value) -> RecalcReport {
        let input = if value.is_empty() { None } else { Some(Input::Literal(value)) };
        let seeds = self.apply_input(cell, input);
        self.recalculate(&seeds)
    }

    pub fn set_formula(&mut self, cell: CellRef, source: &str) -> RecalcReport {
        let seeds = self.apply_input(cell, Some(Input::Formula(Formula::new(source))));
        self.recalculate(&seeds)
    }

    pub fn clear(&mut self, cell: CellRef) -> RecalcReport {
        let seeds = self.apply_input(cell, None);
        self.recalculate(&seeds)
    }

    pub fn recalculate_all(&mut self) -> RecalcReport {
        let mut seeds: Vec<CellRef> = self.cells.keys().copied().collect();
        seeds.sort_unstable();
        self.recalculate(&seeds)
    }

    pub fn fill(&mut self, source: Bounds, target: Bounds) -> RecalcReport {
        // Snapshot source first: overlapping target must not read overwritten cells
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


    fn apply_input(&mut self, cell: CellRef, input: Option<Input>) -> Vec<CellRef> {
        self.graph.clear_precedents(cell);

        match input {
            None => {
                self.cells.remove(&cell);
            }
            Some(input) => {
                let value = match &input {
                    Input::Literal(value) => value.clone(),
                    Input::Formula(_) => Value::Empty,
                };
                self.cells.insert(cell, Cell { input, value });
            }
        }

        if self.is_volatile_cell(cell) {
            self.volatile_cells.insert(cell);
        } else {
            self.volatile_cells.remove(&cell);
        }

        let precedents = self.precedents_of(cell);
        self.graph.set_precedents(cell, &precedents);

        if !self.cells.contains_key(&cell) {
            self.graph.remove_if_isolated(cell);
        }

        // Ranges are watches, not edges; nothing else needs rebuilding here
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

    fn is_volatile_cell(&self, cell: CellRef) -> bool {
        self.cells
            .get(&cell)
            .and_then(|stored| stored.input.as_formula())
            .and_then(|formula| formula.program.as_ref())
            .is_some_and(Program::is_volatile)
    }

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

    pub fn recalculate(&mut self, seeds: &[CellRef]) -> RecalcReport {
        let dirty = if self.volatile_cells.is_empty() {
            self.dirty_closure(seeds)
        } else {
            let mut all = Vec::with_capacity(seeds.len() + self.volatile_cells.len());
            all.extend_from_slice(seeds);
            all.extend(self.volatile_cells.iter().copied());
            self.dirty_closure(&all)
        };
        if dirty.is_empty() {
            return RecalcReport::default();
        }
        let (levels, cycles) = levelize(&self.graph, &dirty);

        for cell in &cycles {
            if let Some(stored) = self.cells.get_mut(cell) {
                stored.value = Value::Error(ErrorKind::Cycle);
            }
        }

        for level in &levels {
            let updates: Vec<(CellRef, Value)> = if level.len() < 2 {
                level.iter().map(|cell| (*cell, self.eval_cell(*cell))).collect()
            } else {
                // Same-level cells are independent; values applied after parallel eval
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

fn shift_input(input: Input, row_delta: i64, col_delta: i64) -> Input {
    match input {
        // Literals copy verbatim; no series detection
        Input::Literal(value) => Input::Literal(value),
        Input::Formula(formula) => {
            let Ok(expr) = crate::parser::parse(&formula.source) else {
                return Input::Formula(formula);
            };
            let text = format!("={}", expr.shifted(row_delta, col_delta));
            Input::Formula(Formula::new(text))
        }
    }
}

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
    // NaN, inf and overflow are not spreadsheet numbers
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

        sheet.set_input(cell("A2"), "4");
        assert_eq!(number(&sheet, "B1"), 8.0);

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
        assert_eq!(report.dirty, vec![cell("A1"), cell("B1"), cell("C1")]);
        assert_eq!(report.recalculated(), 3);
        assert_eq!(sheet.value(cell("B2")), Value::Number(2.0));
    }

    #[test]
    fn editing_an_unrelated_cell_refreshes_volatile_cells() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("D4"), "=TODAY()");
        sheet.set_input(cell("E4"), "=NOW()");
        sheet.set_input(cell("F4"), "=1+1");

        let report = sheet.set_input(cell("A1"), "2");
        assert!(report.dirty.contains(&cell("D4")), "dirty was {:?}", report.dirty);
        assert!(report.dirty.contains(&cell("E4")), "dirty was {:?}", report.dirty);
        assert!(!report.dirty.contains(&cell("F4")), "dirty was {:?}", report.dirty);
        assert_eq!(report.recalculated(), 3);
    }

    #[test]
    fn volatile_cells_propagate_to_their_dependents() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("B1"), "=TODAY()*2");
        sheet.set_input(cell("C1"), "=B1+1");

        let report = sheet.set_input(cell("A1"), "2");
        assert!(report.dirty.contains(&cell("B1")), "dirty was {:?}", report.dirty);
        assert!(report.dirty.contains(&cell("C1")), "dirty was {:?}", report.dirty);
    }

    #[test]
    fn a_volatile_cell_stops_being_seeded_once_it_is_cleared() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("D4"), "=TODAY()");

        sheet.clear(cell("D4"));
        let report = sheet.set_input(cell("A1"), "2");
        assert!(!report.dirty.contains(&cell("D4")), "dirty was {:?}", report.dirty);
        assert_eq!(report.dirty, vec![cell("A1")]);
    }

    #[test]
    fn a_cell_that_stops_calling_a_volatile_function_stops_being_seeded() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("D4"), "=TODAY()");

        sheet.set_input(cell("D4"), "=1+1");
        let report = sheet.set_input(cell("A1"), "2");
        assert_eq!(report.dirty, vec![cell("A1")]);
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

        sheet.set_input(cell("A7"), "10");
        assert_eq!(number(&sheet, "B1"), 13.0);

        sheet.clear(cell("A7"));
        assert_eq!(number(&sheet, "B1"), 3.0);
    }

    #[test]
    fn range_dependencies_are_visible_as_watches() {
        let mut sheet = Sheet::new();
        sheet.set_input(cell("A1"), "1");
        sheet.set_input(cell("B1"), "=SUM(A1:A3)");
        assert_eq!(sheet.precedents(cell("B1")), Vec::new());
        assert_eq!(sheet.dependents(cell("A1")), Vec::new());
        assert_eq!(sheet.range_watchers(cell("A1")), vec![cell("B1")]);
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
