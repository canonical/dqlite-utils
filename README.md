# `dqlite-utils`

`dqlite-utils` is an observability tool for inspecting the on-disk state of a `dqlite` node.
The inspected dqlite node does not have to be stopped and does not have to be the leader.
This allows issues to be debugged without interacting with a live process or the rest of the cluster.

Thus, `dqlite-utils` never affects existing data or server execution.

## Installation

### From crates.io

To use the latest stable version, install directly from `crates.io`:

```bash
cargo install dqlite-utils
```

### From Source

To build from the latest source code:

```bash
git clone https://github.com/canonical/dqlite-utils.git
cd dqlite-utils
cargo install --path .
```

## Usage

This tool opens a shell over the content of a dqlite folder.
By default, it works on the current folder, but you can specify the path to the dqlite data directory like:

```bash
dqlite-utils --dir /path/to/dqlite/data
>
```

To use `dqlite-utils` non-interactively, pass commands via the `-c` flag.
