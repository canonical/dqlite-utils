# Preface

This file is the local knowledge-base index for the `dqlite-utils` binary crate (`crates/dqlite_utils_cli/src/`). It describes the REPL CLI layout and references to the library crate.

# Overview

The binary crate contains the dqlite-utils REPL: CLI argument parsing, interactive command reading, shell dispatch, and user-facing commands. It depends on the `dqlite_utils` library for dqlite storage and SQLite extension types, accessed via `dqlite_utils::dqlite::*` and `dqlite_utils::rusqlite_ext::*`.

# Important

- All `dqlite_utils::` references point to the library crate. Binary-internal types (`Context`, `Shell`, `ShellKind`, `Result`) use `crate::` as usual.
- `CommandHelper` trait is defined in `command/mod.rs` and implemented by `ShellKind` (defined in `main.rs`).
- Tests are colocated under `#[cfg(test)]` blocks in the modules they exercise.

# Directory

- `main.rs` - CLI entry point, `Context`, `Shell`, `ShellKind`, `CommandHelper` impl, and dispatch between batch and interactive execution.
- `args.rs` - Clap-based parsing for top-level process flags.
- `interactive_reader.rs` - Rustyline integration and command completion helpers.
- `prompt.rs` - Prompt rendering for root and nested shells.
- `utils.rs` - Shared terminal and formatting utilities.
- `command/` - User-facing REPL commands and shell-specific command dispatch.

# Index

- `../command/kb/AGENTS.md` - Command and shell structure
- `../../../src/dqlite/kb/AGENTS.md` - Low-level dqlite storage logic (library crate)
- `../../../src/rusqlite_ext/kb/AGENTS.md` - SQLite extension support (library crate)