# Preface

This file covers the `src/rusqlite_ext/` subtree, which provides SQLite-facing extensions and lower-level glue used by the open-shell workflow.

# Overview

This module wraps SQLite result codes, file controls, file abstractions, configuration helpers, and the custom VFS support needed to expose dqlite-backed databases through `rusqlite`.

# Important

- Keep SQLite pointer-level and VFS-specific logic inside this subtree. Command handlers should consume higher-level interfaces instead of reaching into raw `libsqlite3_sys`.
- Result-code conversions are centralized here; preserve those wrappers so errors remain consistent across the VFS and command layers.
- VFS changes are high risk because they affect `.open` behavior and the consistency of reconstructed database state across raft indexes.

# Directory

- `mod.rs` - Shared SQLite result wrappers and extension-layer utilities.
- `config.rs` - SQLite configuration helpers.
- `file_control.rs` - File-control wrappers for SQLite operations.
- `files.rs` - File abstractions used by the VFS layer.
- `vfs.rs` - Custom VFS implementation and registration support.
