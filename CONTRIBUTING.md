# Contributing

Lattice started as a project to see how fast and correct a spreadsheet engine could actually be, so a couple of things below exist to protect that, not because we like rules for their own sake.

## Getting Started

Two crates:

- `engine/`, cell storage, the formula language, the recalculation engine.
- `app/`, the iced desktop front end.

`app` depends on `engine`, never the other way around.

```sh
cargo build --workspace
cargo test --workspace
```

Both should be clean before you push.

## Coding Standards

- No compiler warnings.
- No [clippy](https://github.com/rust-lang/rust-clippy) warnings.
- Format your code with `rustfmt`.
- `engine` never depends on `iced`, `wgpu`, `winit`, `softbuffer`, or anything like them, not even as a dev-dependency. Check it with `cargo tree -p engine | grep -iE 'iced|wgpu|winit|softbuffer'`, it should print nothing.
- Nothing in `engine/src/` imports from `app`. If something only makes sense once there's a screen (selection, viewport, scroll position, status bar text), it belongs in `app/src/state.rs` instead.
- Any change to `engine/src/graph.rs` or `engine/src/sheet.rs` needs a test that proves the work done, not just that it's fast. `RecalcReport` gives you `dirty`, `recalculated()`, `depth()`, `levels`, and `cycles` for exactly this. A timing check on top is fine, but never as the only proof, see `engine/tests/scale.rs` for the pattern.
- Don't delete `engine/tests/properties.proptest-regressions`. It's proptest's memory of past bugs, deleting it just lets old ones sneak back in unnoticed. If proptest adds a line to it while you're fixing something, commit that line along with the fix.

## General Guidelines

- Keep PRs scoped to one thing. A refactor and a behavior change in the same PR make both harder to review.
- If you're changing formula evaluation or dependency-graph behavior, add a test that locks in the new behavior and explain the why in the PR description.
- Welcome contribution areas: new spreadsheet functions, UI polish, performance work, documentation.

## Where the tests live

| file                              | covers                                                         |
| --------------------------------- | -------------------------------------------------------------- |
| `engine/src/*.rs`                 | unit tests at the bottom of each module                        |
| `engine/tests/properties.rs`      | random-input invariants plus the saved regressions             |
| `engine/tests/scale.rs`           | big-sheet behavior, work done rather than time                 |
| `engine/tests/readme_examples.rs` | every example in FUNCTIONS.md, checked against the real engine |
| `app/src/state.rs`                | construction and read-only accessors                           |
| `app/src/input.rs`                | keyboard/pointer handling and the update loop                  |
| `app/src/persistence.rs`          | saving, loading, the naming prompt                             |
| `app/src/settings.rs`             | settings file, following the OS theme                          |
| `app/src/theme.rs`                | the light and dark palettes staying in sync                    |
| `app/src/grid.rs`                 | hit-testing, scrolling, canvas drawing                         |
| `app/src/application.rs`          | no tests, it's just the widget tree                            |

## Minimum Rust Version

- `engine`: 1.88
- `app`, and the workspace overall: 1.90

`iced` needs 1.88, and its `wgpu` backend pulls in `ordered-float`, which needs 1.90. If a dependency bump raises either number, update it in the crate's `rust-version`, the README, and this file, all in the same change.

## Licensing

As mentioned in the [README](README.md), all contributions are licensed under MIT.
