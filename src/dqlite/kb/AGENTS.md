# Preface

This file documents the `src/dqlite/` subsystem, which encapsulates dqlite metadata access, raft log decoding, snapshot handling, and FFI interaction with libdqlite.

# Overview

`src/dqlite/mod.rs` is the low-level storage layer behind the user-facing commands. It loads dqlite metadata from disk, enumerates snapshots and segments, decodes raft commands, and creates synthetic snapshots for export. `sys.rs` exposes bindgen-generated raw types and functions.

# Important

- Prefer `DqliteDir` and related wrappers when reading repository data; do not spread direct libdqlite calls into higher layers.
- Preserve RAII ownership around allocated C resources such as `RaftPtr` and decode buffers. Leaks or double-frees here will not be caught by the type system.
- Changes to `build.rs`, `dqlite-internal.h`, or bindgen-facing types can affect the generated FFI surface and should be reflected in KB notes when they alter maintenance workflow.

# Architecture

This module is intentionally split between unsafe boundary code and safe higher-level wrappers. Metadata loading and segment enumeration happen first, then snapshots and raft entries are decoded into Rust-facing structures consumed by the command layer. Snapshot creation also flows through this module so on-disk format rules remain centralized.

# Directory

- `mod.rs` - Safe wrappers, decoding helpers, snapshot handling, and file-format logic.
- `sys.rs` - Raw bindgen-generated symbols and type definitions used by the wrapper layer.
