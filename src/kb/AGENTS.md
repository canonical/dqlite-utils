# Preface

This file is the local knowledge-base index for `src/`. It explains the top-level Rust module layout for the **library crate** (`dqlite_utils`). The binary crate lives in `crates/dqlite_utils_cli/` and has its own KB index.

# Overview

`src/` contains the `dqlite_utils` library crate. The dqlite directory implementation lives in `dir.rs` (`dqlite_utils::dir::*`), shared raft-facing types now live in `raft.rs` (`dqlite_utils::raft::*`), and `lib.rs` remains a thin crate root that re-exports `DqliteDir` at the top level for now. `rusqlite_ext/` remains a separate public module. `sys.rs` is a private bindgen-backed FFI layer. The binary crate (`dqlite-utils`) depends on this library for all dqlite and rusqlite_ext types.

# Important

- Keep unsafe and low-level storage handling isolated in `sys.rs` and `rusqlite_ext/`; higher-level code should use the existing wrappers instead of direct FFI or SQLite pointer manipulation.
- Preserve RAII ownership around allocated C resources such as `RaftPtr` and decode buffers. Leaks or double-frees here will not be caught by the type system.
- Changes to `build.rs`, `dqlite-internal.h`, or bindgen-facing types can affect the generated FFI surface and should be reflected in KB notes when they alter maintenance workflow.
- Tests are colocated under `#[cfg(test)]` blocks in the modules they exercise. There is no top-level `tests/` directory.
- Rust toolchain `1.91` is pinned in `rust-toolchain.toml` and CI runs `fmt`, `check`, `clippy`, `doc`, and both debug and release test suites.
- The library and binary are in a Cargo workspace. The library crate is at the repo root and the binary crate is in `crates/dqlite_utils_cli/`. Both share metadata (version, edition, rust-version) via `workspace.package` and shared dependencies via `workspace.dependencies`.

# Architecture

The dqlite implementation is intentionally split between unsafe boundary code (`sys.rs`) and safe higher-level wrappers in `dir.rs` and `raft.rs`. The `raft.rs` module owns shared raft configuration/error/resource helpers, while `dir.rs` handles metadata loading, segment enumeration, snapshot decoding, and on-disk creation. The binary crate consumes both layers depending on whether it needs raft metadata types or dqlite directory traversal.

# Directory

- `lib.rs` - Thin library crate root that wires modules together and re-exports `DqliteDir`.
- `dir.rs` - Dqlite directory implementation: metadata loading, raft log parsing, snapshots, builders, and colocated tests.
- `raft.rs` - Shared raft-facing types and helpers: configuration, roles, servers, error string, and C-owned pointer wrapper.
- `sys.rs` - Raw bindgen-generated symbols and type definitions used by the dqlite implementation (private module).
- `rusqlite_ext/` - SQLite extension helpers, VFS abstractions, and file-control wrappers.

# Index

- `../rusqlite_ext/kb/AGENTS.md` - SQLite extension support
- `../../crates/dqlite_utils_cli/src/kb/AGENTS.md` - Binary crate layout
