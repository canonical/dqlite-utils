# Preface

This file documents the `snap/` directory, which contains the Snap packaging configuration for `dqlite-utils`.

# Overview

The snap packaging builds the Rust binary from this source tree, pulls build dependencies from the `dqlite/dev` PPA, and packages the resulting CLI as a strictly confined `core24` snap.

# Important

- Keep `snap/snapcraft.yaml` aligned with Rust toolchain and dependency expectations from CI when packaging changes are made.
- Packaging depends on `libdqlite1.18-unstable-dev` plus development packages for SQLite, LZ4, and libuv; changes in native linking requirements should be reflected here.

# Directory

- `snapcraft.yaml` - Snap metadata, build dependencies, and the Rust build definition.
