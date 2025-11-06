# `dqlite-utils`

A CLI for passive, read-only inspection and postmortem debugging of dqlite nodes.

## Overview

This utility operates directly on the on-disk state of a `dqlite` node, parsing the Raft segments, snapshots, and metadata files. This approach means it's completely passive and safe to run on a malfunctioning or even a completely stopped node. You can confidently debug issues without ever interacting with a live process or the rest of the cluster.

## Installation

### From crates.io

For the latest stable version, you can install directly from crates.io:

```bash
cargo install dqlite-utils
```

### From Source

To build from the latest source code:

```bash
# Clone the repository
git clone https://github.com/canonical/dqlite-utils.git
cd dqlite-utils

# Build and install
cargo install --path .
```

## Usage

This tool opens an interactive shell to inspect the contents of a dqlite folder. It works on the current folder by default, but you can specify the path to the dqlite data directory like:

```bash
dqlite-utils --dir /path/to/dqlite/data
>
```

It is also possible to run in non-interactive mode by using `-c <COMMANDS>`. It is possible to both specify commands separated by `;` or by using multiple `-c` flags.
