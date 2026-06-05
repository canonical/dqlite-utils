# Preface

This file is the local knowledge-base index for `src/`. It explains the top-level Rust module layout for the **library crate** (`dqlite_utils`). The binary crate lives in `crates/dqlite_utils_cli/` and has its own KB index.

# Overview

`src/` contains the `dqlite_utils` library crate, which exposes `dqlite/` and `rusqlite_ext/` as public modules. The library provides dqlite metadata loading, raft log parsing, snapshot handling, and SQLite extension helpers. The binary crate (`dqlite-utils`) depends on this library for all dqlite and rusqlite_ext types.

# Important

- Keep unsafe and low-level storage handling isolated in `dqlite/` and `rusqlite_ext/`; higher-level code should use the existing wrappers instead of direct FFI or SQLite pointer manipulation.
- Tests are colocated under `#[cfg(test)]` blocks in the modules they exercise. There is no top-level `tests/` directory.
- Rust toolchain `1.91` is pinned in `rust-toolchain.toml` and CI runs `fmt`, `check`, `clippy`, `doc`, and both debug and release test suites.
- The library and binary are in a Cargo workspace. The library crate is at the repo root and the binary crate is in `crates/dqlite_utils_cli/`. Both share metadata (version, edition, rust-version) via `workspace.package` and shared dependencies via `workspace.dependencies`.

# Directory

- `lib.rs` - Library crate root: `pub mod dqlite; pub mod rusqlite_ext;`
- `dqlite/` - Dqlite metadata loading, raft log parsing, snapshot handling, and bindgen-backed FFI.
- `rusqlite_ext/` - SQLite extension helpers, VFS abstractions, and file-control wrappers.

# Index

- `../dqlite/kb/AGENTS.md` - Low-level dqlite storage logic
- `../rusqlite_ext/kb/AGENTS.md` - SQLite extension support
- `../../crates/dqlite_utils_cli/src/kb/AGENTS.md` - Binary crate layout