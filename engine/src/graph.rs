use std::collections::{HashMap, HashSet};

use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::addr::{CellRef, RangeRef};
use crate::compile::Precedent;

#[derive(Clone, Debug, Default)]
pub struct DepGraph {
    graph: StableGraph<CellRef, ()>,
    nodes: HashMap<CellRef, NodeIndex>,
    watches: Vec<(RangeRef, CellRef)>,
}

impl DepGraph {
    pub fn new() -> Self {
        DepGraph::default()
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    fn ensure_node(&mut self, cell: CellRef) -> NodeIndex {
        match self.nodes.get(&cell) {
            Some(index) => *index,
            None => {
                let index = self.graph.add_node(cell);
                self.nodes.insert(cell, index);
                index
            }
        }
    }

    fn node(&self, cell: CellRef) -> Option<NodeIndex> {
        self.nodes.get(&cell).copied()
    }

    pub fn dependents(&self, cell: CellRef) -> Vec<CellRef> {
        let Some(index) = self.node(cell) else {
            return Vec::new();
        };
        let mut out: Vec<CellRef> = self
            .graph
            .neighbors_directed(index, Direction::Outgoing)
            .map(|n| self.graph[n])
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn precedents(&self, cell: CellRef) -> Vec<CellRef> {
        let Some(index) = self.node(cell) else {
            return Vec::new();
        };
        let mut out: Vec<CellRef> = self
            .graph
            .neighbors_directed(index, Direction::Incoming)
            .map(|n| self.graph[n])
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    // formulas whose range covers cell, even with no edge
    pub fn range_watchers(&self, cell: CellRef) -> Vec<CellRef> {
        let mut out: Vec<CellRef> = self
            .watches
            .iter()
            .filter(|(range, _)| range.contains(cell))
            .map(|(_, formula)| *formula)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn clear_precedents(&mut self, cell: CellRef) {
        self.watches.retain(|(_, formula)| *formula != cell);
        let Some(index) = self.node(cell) else {
            return;
        };
        let sources: Vec<CellRef> = self
            .graph
            .neighbors_directed(index, Direction::Incoming)
            .map(|node| self.graph[node])
            .collect();
        let mut edges: Vec<_> = self
            .graph
            .edges_directed(index, Direction::Incoming)
            .map(|edge| edge.id())
            .collect();
        edges.sort_unstable();
        edges.dedup();
        for edge in edges {
            self.graph.remove_edge(edge);
        }
        for source in sources {
            self.remove_if_isolated(source);
        }
    }

    pub fn set_precedents(&mut self, cell: CellRef, precedents: &[Precedent]) {
        self.clear_precedents(cell);
        if precedents.is_empty() {
            return;
        }
        let dependent = self.ensure_node(cell);
        for precedent in precedents {
            match precedent {
                Precedent::Cell(source) => {
                    let source = self.ensure_node(*source);
                    self.add_edge(source, dependent);
                }
                Precedent::Range(range) => self.watches.push((*range, cell)),
            }
        }
    }

    fn add_edge(&mut self, from: NodeIndex, to: NodeIndex) {
        if !self.graph.contains_edge(from, to) {
            self.graph.add_edge(from, to, ());
        }
    }

    // Graph::edges yields only outgoing; check both directions
    pub fn remove_if_isolated(&mut self, cell: CellRef) {
        let Some(index) = self.node(cell) else {
            return;
        };
        let has_outgoing = self.graph.edges(index).next().is_some();
        let has_incoming = self.graph.edges_directed(index, Direction::Incoming).next().is_some();
        let watched = self.watches.iter().any(|(_, formula)| *formula == cell);
        if !has_outgoing && !has_incoming && !watched {
            self.graph.remove_node(index);
            self.nodes.remove(&cell);
        }
    }
}

// returns level groups plus the cells sitting on cycles
pub fn levelize(graph: &DepGraph, dirty: &[CellRef]) -> (Vec<Vec<CellRef>>, Vec<CellRef>) {
    let members: HashSet<CellRef> = dirty.iter().copied().collect();


    let mut dependents: HashMap<CellRef, Vec<CellRef>> = HashMap::new();
    for &cell in dirty {
        for precedent in graph.precedents(cell) {
            if members.contains(&precedent) {
                dependents.entry(precedent).or_default().push(cell);
            }
        }
        for dependent in graph.range_watchers(cell) {
            if members.contains(&dependent) {
                dependents.entry(cell).or_default().push(dependent);
            }
        }
    }
    for list in dependents.values_mut() {
        list.sort_unstable();
        list.dedup();
    }

    let mut pending: HashMap<CellRef, usize> = dirty.iter().map(|cell| (*cell, 0)).collect();
    for list in dependents.values() {
        for dependent in list {
            if let Some(count) = pending.get_mut(dependent) {
                *count += 1;
            }
        }
    }

    let mut queue: Vec<CellRef> =
        dirty.iter().copied().filter(|cell| pending[cell] == 0).collect();
    queue.sort_unstable();

    let mut levels: Vec<Vec<CellRef>> = Vec::new();
    let mut cycles: Vec<CellRef> = Vec::new();
    let mut done: HashSet<CellRef> = HashSet::new();

    while done.len() < dirty.len() {
        if queue.is_empty() {
            let unresolved: HashSet<CellRef> =
                dirty.iter().copied().filter(|cell| !done.contains(cell)).collect();
            let on_cycles = cycle_members(&unresolved, &dependents);
            if on_cycles.is_empty() {
                debug_assert!(false, "levelize stalled with no cycle in the dirty subgraph");
                break;
            }
            for cell in &on_cycles {
                done.insert(*cell);
            }
            for cell in &on_cycles {
                release(*cell, &dependents, &mut pending, &done, &mut queue);
            }
            cycles.extend(on_cycles);
            continue;
        }

        let level = std::mem::take(&mut queue);
        for cell in &level {
            done.insert(*cell);
        }
        for cell in &level {
            release(*cell, &dependents, &mut pending, &done, &mut queue);
        }
        queue.sort_unstable();
        queue.dedup();
        levels.push(level);
    }

    cycles.sort_unstable();
    (levels, cycles)
}

fn release(
    cell: CellRef,
    dependents: &HashMap<CellRef, Vec<CellRef>>,
    pending: &mut HashMap<CellRef, usize>,
    done: &HashSet<CellRef>,
    queue: &mut Vec<CellRef>,
) {
    for dependent in dependents.get(&cell).into_iter().flatten() {
        let Some(count) = pending.get_mut(dependent) else {
            continue;
        };
        *count = count.saturating_sub(1);
        if *count == 0 && !done.contains(dependent) {
            queue.push(*dependent);
        }
    }
}

// on-cycle cells are SCCs of size >1 or self-loops
fn cycle_members(
    remaining: &HashSet<CellRef>,
    dependents: &HashMap<CellRef, Vec<CellRef>>,
) -> Vec<CellRef> {
    let mut nodes: Vec<CellRef> = remaining.iter().copied().collect();
    nodes.sort_unstable();
    let index: HashMap<CellRef, usize> =
        nodes.iter().enumerate().map(|(position, cell)| (*cell, position)).collect();

    let adjacency: Vec<Vec<usize>> = nodes
        .iter()
        .map(|cell| {
            let mut out: Vec<usize> = dependents
                .get(cell)
                .into_iter()
                .flatten()
                .filter_map(|dependent| index.get(dependent).copied())
                .collect();
            out.sort_unstable();
            out.dedup();
            out
        })
        .collect();

    let mut on_cycles: Vec<CellRef> = Vec::new();
    for component in strongly_connected_components(adjacency.len(), &adjacency) {
        let self_referencing =
            component.len() == 1 && adjacency[component[0]].contains(&component[0]);
        if component.len() > 1 || self_referencing {
            on_cycles.extend(component.into_iter().map(|position| nodes[position]));
        }
    }
    on_cycles.sort_unstable();
    on_cycles
}

// iterative Tarjan: deep chains must not overflow the stack
fn strongly_connected_components(node_count: usize, adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const UNVISITED: usize = usize::MAX;

    let mut index = vec![UNVISITED; node_count];
    let mut low_link = vec![0usize; node_count];
    let mut on_stack = vec![false; node_count];
    let mut component_stack: Vec<usize> = Vec::new();
    let mut next_index = 0usize;
    let mut components: Vec<Vec<usize>> = Vec::new();
    let mut frames: Vec<(usize, usize)> = Vec::new();

    for root in 0..node_count {
        if index[root] != UNVISITED {
            continue;
        }
        index[root] = next_index;
        low_link[root] = next_index;
        next_index += 1;
        component_stack.push(root);
        on_stack[root] = true;
        frames.push((root, 0));

        while let Some(frame) = frames.last().copied() {
            let (node, next_child) = frame;
            if next_child < adjacency[node].len() {
                frames.last_mut().expect("frame exists").1 += 1;
                let child = adjacency[node][next_child];
                if index[child] == UNVISITED {
                    index[child] = next_index;
                    low_link[child] = next_index;
                    next_index += 1;
                    component_stack.push(child);
                    on_stack[child] = true;
                    frames.push((child, 0));
                } else if on_stack[child] {
                    low_link[node] = low_link[node].min(index[child]);
                }
            } else {
                frames.pop();
                if low_link[node] == index[node] {
                    let mut component = Vec::new();
                    while let Some(member) = component_stack.pop() {
                        on_stack[member] = false;
                        component.push(member);
                        if member == node {
                            break;
                        }
                    }
                    components.push(component);
                }
                if let Some(parent) = frames.last().copied() {
                    low_link[parent.0] = low_link[parent.0].min(low_link[node]);
                }
            }
        }
    }

    components
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Ref;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse_a1(a1).unwrap()
    }

    fn range(a1: &str, b1: &str) -> RangeRef {
        RangeRef { start: Ref::parse(a1).unwrap(), end: Ref::parse(b1).unwrap() }
    }

    #[test]
    fn records_precedents_and_dependents() {
        let mut graph = DepGraph::new();
        let b1 = cell("B1");
        graph.set_precedents(b1, &[Precedent::Cell(cell("A1"))]);
        assert_eq!(graph.precedents(b1), vec![cell("A1")]);
        assert_eq!(graph.dependents(cell("A1")), vec![b1]);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn re_editing_a_cell_replaces_its_precedents() {
        let mut graph = DepGraph::new();
        let c1 = cell("C1");
        graph.set_precedents(c1, &[Precedent::Cell(cell("A1"))]);
        graph.set_precedents(c1, &[Precedent::Cell(cell("B1"))]);
        assert_eq!(graph.precedents(c1), vec![cell("B1")]);
        assert!(graph.dependents(cell("A1")).is_empty());
        graph.remove_if_isolated(cell("A1"));
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn ranges_become_a_watch_not_a_thousand_edges() {
        let mut graph = DepGraph::new();
        let total = cell("C1");
        let span = range("A1", "A10");
        graph.set_precedents(total, &[Precedent::Range(span)]);
        assert_eq!(graph.edge_count(), 0);
        assert_eq!(graph.node_count(), 1);
        assert!(graph.precedents(total).is_empty());
        for a1 in ["A1", "A5", "A10"] {
            assert_eq!(graph.range_watchers(cell(a1)), vec![total]);
        }
        assert!(graph.range_watchers(cell("A11")).is_empty());
        assert!(graph.range_watchers(cell("B7")).is_empty());
    }

    #[test]
    fn range_dependencies_still_order_and_cycle() {
        let mut graph = DepGraph::new();
        let (a1, a2) = (cell("A1"), cell("A2"));
        graph.set_precedents(a2, &[Precedent::Range(range("A1", "A3"))]);
        let (levels, cycles) = levelize(&graph, &[a1, a2]);
        assert_eq!(cycles, vec![a2]);
        assert_eq!(levels, vec![vec![a1]]);

        let mut graph = DepGraph::new();
        let (b1, c1) = (cell("B1"), cell("C1"));
        graph.set_precedents(c1, &[Precedent::Range(range("B1", "B3"))]);
        let (levels, cycles) = levelize(&graph, &[b1, c1]);
        assert_eq!(cycles, Vec::new());
        assert_eq!(levels, vec![vec![b1], vec![c1]]);
    }

    #[test]
    fn levelize_orders_dependencies_and_parallelises_independent_cells() {
        let mut graph = DepGraph::new();
        let (a1, b1, c1, d1) = (cell("A1"), cell("B1"), cell("C1"), cell("D1"));
        graph.set_precedents(b1, &[Precedent::Cell(a1)]);
        graph.set_precedents(c1, &[Precedent::Cell(b1)]);
        graph.set_precedents(d1, &[Precedent::Cell(a1)]);

        let (levels, cycles) = levelize(&graph, &[a1, b1, c1, d1]);
        assert_eq!(levels, vec![vec![a1], vec![b1, d1], vec![c1]]);
        assert!(cycles.is_empty());
    }

    #[test]
    fn non_dirty_precedents_do_not_delay_a_level() {
        let mut graph = DepGraph::new();
        let (a1, b1) = (cell("A1"), cell("B1"));
        graph.set_precedents(b1, &[Precedent::Cell(a1)]);
        let (levels, cycles) = levelize(&graph, &[b1]);
        assert_eq!(levels, vec![vec![b1]]);
        assert!(cycles.is_empty());
    }

    #[test]
    fn detects_a_two_cell_cycle() {
        let mut graph = DepGraph::new();
        let (a1, b1) = (cell("A1"), cell("B1"));
        graph.set_precedents(a1, &[Precedent::Cell(b1)]);
        graph.set_precedents(b1, &[Precedent::Cell(a1)]);
        let (levels, cycles) = levelize(&graph, &[a1, b1]);
        assert!(levels.is_empty());
        assert_eq!(cycles, vec![a1, b1]);
    }

    #[test]
    fn detects_a_self_reference() {
        let mut graph = DepGraph::new();
        let a1 = cell("A1");
        graph.set_precedents(a1, &[Precedent::Cell(a1)]);
        let (levels, cycles) = levelize(&graph, &[a1]);
        assert!(levels.is_empty());
        assert_eq!(cycles, vec![a1]);
    }

    #[test]
    fn only_the_cycle_members_are_reported_not_their_dependents() {
        let mut graph = DepGraph::new();
        let (a1, b1, c1) = (cell("A1"), cell("B1"), cell("C1"));
        graph.set_precedents(a1, &[Precedent::Cell(b1)]);
        graph.set_precedents(b1, &[Precedent::Cell(a1)]);
        graph.set_precedents(c1, &[Precedent::Cell(b1)]);

        let (levels, cycles) = levelize(&graph, &[a1, b1, c1]);
        assert_eq!(cycles, vec![a1, b1]);
        assert_eq!(levels, vec![vec![c1]]);
    }

    #[test]
    fn a_cell_downstream_of_a_cycle_is_not_part_of_it() {
        let mut graph = DepGraph::new();
        let (a1, b1, a2, b2, c2) = (cell("A1"), cell("B1"), cell("A2"), cell("B2"), cell("C2"));
        graph.set_precedents(b1, &[Precedent::Cell(a1), Precedent::Cell(b1)]);
        graph.set_precedents(a2, &[Precedent::Cell(a1), Precedent::Cell(b1)]);
        graph.set_precedents(b2, &[Precedent::Cell(a1), Precedent::Cell(a2)]);
        graph.set_precedents(c2, &[Precedent::Cell(b1)]);

        let (levels, cycles) = levelize(&graph, &[a1, b1, a2, b2, c2]);
        assert_eq!(cycles, vec![b1], "only B1 is on a cycle");
        assert_eq!(levels, vec![vec![a1], vec![a2, c2], vec![b2]]);
    }

    #[test]
    fn every_cycle_is_reported_even_with_several_of_them() {
        let mut graph = DepGraph::new();
        let (a1, b1, c1, d1) = (cell("A1"), cell("B1"), cell("C1"), cell("D1"));
        graph.set_precedents(a1, &[Precedent::Cell(b1)]);
        graph.set_precedents(b1, &[Precedent::Cell(a1)]);
        graph.set_precedents(c1, &[Precedent::Cell(c1)]);
        graph.set_precedents(d1, &[Precedent::Cell(a1)]);
        let (levels, cycles) = levelize(&graph, &[a1, b1, c1, d1]);
        assert_eq!(cycles, vec![a1, b1, c1]);
        assert_eq!(levels, vec![vec![d1]]);
    }

    #[test]
    fn strongly_connected_components_handles_self_loops_and_chains() {
        let adjacency = vec![vec![1], vec![0], vec![3], vec![], vec![4], vec![]];
        let mut components = strongly_connected_components(6, &adjacency);
        for component in components.iter_mut() {
            component.sort_unstable();
        }
        components.sort();
        assert_eq!(components, vec![vec![0, 1], vec![2], vec![3], vec![4], vec![5]]);

        let n = 50_000;
        let chain: Vec<Vec<usize>> = (0..n).map(|i| if i + 1 < n { vec![i + 1] } else { vec![] }).collect();
        let components = strongly_connected_components(n, &chain);
        assert_eq!(components.len(), n);
    }

    #[test]
    fn a_three_cell_cycle_with_an_entry_point() {
        let mut graph = DepGraph::new();
        let (x1, a1, b1, c1) = (cell("X1"), cell("A1"), cell("B1"), cell("C1"));
        graph.set_precedents(a1, &[Precedent::Cell(x1), Precedent::Cell(c1)]);
        graph.set_precedents(b1, &[Precedent::Cell(a1)]);
        graph.set_precedents(c1, &[Precedent::Cell(b1)]);

        let (levels, cycles) = levelize(&graph, &[x1, a1, b1, c1]);
        assert_eq!(cycles, vec![a1, b1, c1]);
        assert_eq!(levels, vec![vec![x1]]);
    }

    #[test]
    fn levels_are_deterministic() {
        let mut graph = DepGraph::new();
        let cells: Vec<CellRef> = (0..8).map(|i| CellRef::new(0, i)).collect();
        for pair in cells.windows(2) {
            graph.set_precedents(pair[1], &[Precedent::Cell(pair[0])]);
        }
        let first = levelize(&graph, &cells);
        let second = levelize(&graph, &cells);
        assert_eq!(first, second);
        assert_eq!(first.0.len(), 8);
    }
}
