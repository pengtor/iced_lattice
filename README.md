# Lattice

A spreadsheet with a real recalculation engine and a desktop UI built in Rust with iced.

<!--
SCREENSHOT: the app window, full size, showing a small filled-in sheet.
Good contents: a few columns of numbers, a formula cell selected with the
formula bar showing something like =SUM(A1:A4), and the status bar visible
at the bottom. This is the first thing people see, it should make it obvious
at a glance that this is a spreadsheet.
-->

## Why

Most spreadsheets recalculate more than they need to, and most custom-built ones get slow once the sheet gets big. Lattice is built around two goals:

- editing one cell should only recompute the cells that actually depend on it, even in a sheet with 100,000+ rows
- the grid should scroll smoothly no matter how big the sheet is, since it only draws what's visible on screen

Both of these hold up in practice. Editing a cell in a 100,000 row sheet takes about 1.2ms and recomputes exactly 2 cells, no matter where in the sheet you edit.

## How it works, briefly

- `engine/` is the actual spreadsheet logic. cell storage, the formula language, and the recalculation engine. no UI code at all, fully testable on its own
- `app/` is the iced desktop app that displays it

Formulas get parsed into an AST, compiled into a flat bytecode, and evaluated on a stack machine instead of walking a tree every time. Cells are stored sparsely, so an empty cell costs nothing.

The main trick behind the speed: when a formula reads a range like `A1:A1000`, it doesn't create a thousand individual dependencies. It just watches the range. This is what keeps edits fast even on huge sheets, instead of every edit having to rewrite a huge pile of dependency edges.

Circular references get detected and marked as errors instead of freezing or crashing.

<!--
GIF: short loop, maybe 5-8 seconds. Type a formula into a cell (something
like =SUM(C2:C4)), hit enter, then drag the fill handle down a couple rows
and show the references updating. This is the easiest way to show it
actually behaves like a spreadsheet without anyone reading a word of text.
-->

## What it supports right now

- basic arithmetic with normal precedence and parentheses
- cell references (`A1`) and ranges (`A1:B10`)
- absolute references (`$A$1`) that behave correctly when copied or filled
- functions: `SUM`, `AVERAGE`, `COUNT`, `IF`, `MIN`, `MAX`, `CONCAT`
- errors as values (`#DIV/0!`, `#REF!`, `#CYCLE!`, etc) instead of crashes
- fill handle, drag to select, formula bar, keyboard navigation
- saving and loading workbooks as JSON

## What it doesn't support yet

- multiple sheets in one workbook
- cell formatting, borders, column widths
- undo/redo
- `.xlsx` import or export
- a native file picker (you name your workbook instead of browsing for a file)

These are all things I plan to add. Contributions welcome.

<!--
GIF or SCREENSHOT (optional): scrolling through a big sheet fast, like
holding Ctrl+Down through a hundred thousand rows, to show it doesn't
stutter. Not essential but a nice one to have if there's time.
-->

## Building it

```sh
cargo test --workspace     # run the test suite
cargo run --release -p app # run the app
```

If you're on NixOS, use `./run.sh` instead. iced and wgpu need some system libraries that NixOS keeps out of the normal search path, and the script sets that up for you.

## Project layout

```
engine/   the spreadsheet engine, no UI dependencies
app/      the iced desktop app
run.sh    NixOS launcher script
```

## License

TBD

## Contributing

More docs on the internals (the dependency graph, the formula compiler, etc) are coming. If you want to dig in before that, `engine/` is fully covered by tests, so it's a reasonably safe place to poke around.
