//! Lattice spreadsheet engine.
//!
//! No UI dependencies: this crate can be embedded, tested and benchmarked on its
//! own. The pipeline is
//!
//! ```text
//! text ──logos──▶ tokens ──chumsky──▶ AST ──compile──▶ flat RPN ──eval──▶ Value
//! ```
//!
//! and [`Sheet`] ties it together with a sparse cell store and a dependency graph
//! that drives topological, incrementally dirty-only recalculation.

pub mod addr;
pub mod ast;
pub mod compile;
pub mod error;
pub mod eval;
pub mod functions;
pub mod graph;
pub mod io;
pub mod lexer;
pub mod parser;
pub mod sheet;
pub mod value;

pub use addr::{Bounds, CellRef, RangeRef, Ref, MAX_COLS, MAX_ROWS};
pub use ast::{BinOp, Expr, UnOp};
pub use compile::{compile, compile_source, FuncId, Op, Precedent, Program};
pub use eval::{evaluate, Operand, ValueSource};
pub use graph::DepGraph;
pub use sheet::{parse_literal, Cell, Formula, Input, RecalcReport, Sheet};
pub use lexer::{lex, Token};
pub use parser::parse;
pub use error::{Diagnostic, ErrorKind};
pub use value::{format_number, Value};
