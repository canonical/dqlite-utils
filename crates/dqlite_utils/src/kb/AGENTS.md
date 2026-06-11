# Preface

This file is the local knowledge-base index for `crates/dqlite_utils/src/`. It explains the top-level Rust module layout for the `dqlite_utils` library crate. The binary crate now lives at the repo root and has its own KB index.

# Overview

`crates/dqlite_utils/src/` contains the `dqlite_utils` library crate. The dqlite directory implementation lives in `dir.rs` (`dqlite_utils::dir::*`), shared raft-facing types live in `raft.rs` (`dqlite_utils::raft::*`), and `lib.rs` remains a thin crate root that re-exports `DqliteDir` at the top level. `sys.rs` is a private bindgen-backed FFI layer. The repo-root binary crate (`dqlite-utils`) depends on this library for dqlite storage types, and also depends separately on `dqlite_utils_rusqlite_ext`.

# Important

- Keep unsafe and low-level storage handling isolated in `sys.rs`; higher-level code should use the existing wrappers instead of direct FFI or SQLite pointer manipulation.
- Preserve RAII ownership around allocated C resources such as `RaftPtr` and decode buffers. Leaks or double-frees here will not be caught by the type system.
- Changes to `build.rs`, `dqlite-internal.h`, or bindgen-facing types can affect the generated FFI surface and should be reflected in KB notes when they alter maintenance workflow.
- Tests are colocated under `#[cfg(test)]` blocks in the modules they exercise. There is no top-level `tests/` directory.
- Rust toolchain `1.91` is pinned in `rust-toolchain.toml` and CI runs `fmt`, `check`, `clippy`, `doc`, and both debug and release test suites.
- The library and binary are in a Cargo workspace. The library crate is in `crates/dqlite_utils/`, the SQLite extension crate is in `crates/dqlite_utils_rusqlite_ext/`, and the binary crate is at the repo root. They share metadata (version, edition, rust-version) via `workspace.package` and shared dependencies via `workspace.dependencies`.

# Architecture

The dqlite implementation is intentionally split between unsafe boundary code (`sys.rs`) and safe higher-level wrappers in `dir.rs` and `raft.rs`. The `raft.rs` module owns shared raft configuration/error/resource helpers, while `dir.rs` handles metadata loading, segment enumeration, snapshot decoding, and on-disk creation. The binary crate consumes both layers depending on whether it needs raft metadata types or dqlite directory traversal.

# Directory

- `lib.rs` - Thin library crate root that wires modules together and re-exports `DqliteDir`.
- `dir.rs` - Dqlite directory implementation: metadata loading, raft log parsing, snapshots, builders, and colocated tests.
- `raft.rs` - Shared raft-facing types and helpers: configuration, roles, servers, error string, and C-owned pointer wrapper.
- `sys.rs` - Raw bindgen-generated symbols and type definitions used by the dqlite implementation (private module).

# Index

- `../../dqlite_utils_rusqlite_ext/src/kb/AGENTS.md` - SQLite extension support
- `../../../src/kb/AGENTS.md` - Binary crate layout
