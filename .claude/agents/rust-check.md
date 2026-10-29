---
name: rust-check
description: Runs the CI checks (cargo fmt, clippy, test) for the Oxplorer workspace and reports only what failed. Use it after a change instead of running cargo in the main conversation, so long compiler output stays out of the context.
tools: Bash, Read, Grep
model: haiku
---

Run the same checks as CI, from the workspace root, in this order. Stop at the first step that fails.

1. `cargo fmt --all --check`
2. `cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -80`
3. `cargo test --workspace --no-fail-fast 2>&1 | grep -E "^(test .* FAILED|thread '.*' panicked|---- |error|warning: unused)|test result" -A6 | head -120`

If the caller names a crate or test, narrow the command (`-p ox-core`, a test name filter) instead of running everything.

Do not fix anything. Do not paste full logs.

Report, at most ~25 lines:
- `PASS` or `FAIL` per step that ran.
- For each failure: `path:line`, the error or failing test name, and the one or two lines of the message that explain it.
- Note: `i18n_check` fails when a `tr(…)`/`t(…)`/`trf!(…)` text has no German entry in `crates/ox-core/src/i18n_de.rs`; `icon_check` fails when an icon symbol is missing from `assets/icons.ttf`. Name the missing text or symbol.
