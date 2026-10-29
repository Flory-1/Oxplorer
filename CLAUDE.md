# Oxplorer

A file explorer for Windows in Rust (egui/eframe). Cargo workspace, toolchain pinned in `rust-toolchain.toml`.

## Layout
- `crates/ox-core/src/`: UI-independent logic, unit tested. Transfer engine in `transfer/`, search and drive index in `search.rs`/`index.rs`, git in `git.rs`, settings in `settings.rs`, translations in `i18n.rs` + `i18n_de.rs` (German table, continued in `i18n_de/`).
- `crates/ox-app/src/`: the app. `app.rs` app state and frame loop (`app/`: `action.rs` the Action enum, `process.rs` dispatcher, `input.rs` keyboard, `poll.rs` per-frame polling, `window.rs` chrome), `tabs.rs` tabs/panes, `ui/<area>.rs` draws one area each, `icons.rs` icon font, `terminal.rs`, `thumbs.rs`.
- `installer/` (Inno Setup, winget), `tools/icon-font/` (builds `assets/icons.ttf`).

## Checks (same as CI)
`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo test --workspace`
Prefer the `rust-check` agent: it runs these and returns only the failures.

## Rules
- **Line budget: no file over 1000 lines.** A hook reports files that cross it. Files already over it (`ui/preview.rs`, `ui/header.rs`, `tabs.rs`) must not grow; put new code in a submodule, or use the `file-splitter` agent first.
- Interface texts go through `tr(…)`/`t(…)`/`trf!(…)` with the English text as the key. Each needs a German entry in `i18n_de.rs` (the `translator` agent adds them). New source files with texts must be added to `SOURCES` in `crates/ox-app/src/i18n_check.rs`.
- Icons are symbols in strings, drawn by `assets/icons.ttf`; a new symbol must be added to `tools/icon-font/build_icon_font.py` (checked by `icon_check.rs`).
- Keep core logic in `ox-core` (testable); `ox-app` only draws and wires things up.
- Note user-visible changes in `CHANGELOG.md`.
- **After every commit, push and open a PR** (`gh pr create`); commit on a branch, never on `main`. If the branch already has a PR, the push updates it.

## Saving tokens
- Never read `target/`, `Cargo.lock`, `i18n_de.rs` or `i18n_de/` in full; grep them.
- For large files, grep for the item and read only that line range.
- Pipe cargo output through `tail`/`grep` instead of printing it all.
- Use the `explorer` agent for searches that span many files.
