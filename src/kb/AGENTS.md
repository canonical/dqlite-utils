# Preface

This file is the local knowledge-base index for `src/`. It explains the top-level Rust module layout and the development workflow expectations that apply across the source tree.

# Overview

`src/` contains a single Rust CLI binary that opens a REPL for inspecting dqlite on-disk state. Entry-point flow starts in `main.rs`, parses CLI arguments, opens a `DqliteDir`, and then dispatches either interactive shell commands or batched commands.

# Important

- Keep unsafe and low-level storage handling isolated in `dqlite/` and `rusqlite_ext/`; higher-level command code should use the existing wrappers instead of direct FFI or SQLite pointer manipulation.
- Tests are colocated under `#[cfg(test)]` blocks in the modules they exercise. There is no top-level `tests/` directory.
- Rust toolchain `1.91` is pinned in `rust-toolchain.toml` and CI runs `fmt`, `check`, `clippy`, `doc`, and both debug and release test suites.

# Directory

- `main.rs` - CLI entry point, shell loop, `Context`, and dispatch between batch and interactive execution.
- `args.rs` - Clap-based parsing for top-level process flags.
- `interactive_reader.rs` - Rustyline integration and command completion helpers.
- `prompt.rs` - Prompt rendering for root and nested shells.
- `utils.rs` - Shared terminal and formatting utilities.
- `command/` - User-facing REPL commands and shell-specific command dispatch.
- `dqlite/` - Dqlite metadata loading, raft log parsing, snapshot handling, and bindgen-backed FFI.
- `rusqlite_ext/` - SQLite extension helpers, VFS abstractions, and file-control wrappers.

# Index

- `../command/kb/AGENTS.md` - Command and shell structure
- `../dqlite/kb/AGENTS.md` - Low-level dqlite storage logic
- `../rusqlite_ext/kb/AGENTS.md` - SQLite extension support
