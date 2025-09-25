# `dqlite-utils`

A CLI for passive, read-only inspection and post-mortem debugging of dqlite nodes.

## Overview

This utility operates directly on the on-disk state of a `dqlite` node, parsing the Raft segments, snapshots, and metadata files. This approach means it's completely passive and safe to run on a malfunctioning or even a completely stopped node. You can confidently debug issues without ever interacting with a live process or the rest of the cluster.

## Features

- **Safe & Passive**: reads data directly from the disk without interacting with the dqlite process.
- **Post-Mortem Analysis**: can debug failed nodes when the cluster is down.
- **Cluster State Overview**: a high-level summary of the Raft state with the `info` command.
- **Detailed Log Inspection**: view the history of Raft entries in a git log-like format using the `log` command.
- **Point-in-Time Querying**: open a read-only SQLite shell for a specific database at any given Raft index with the `open` command.

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

All commands work on the current folder by default, but you can specify the path to the dqlite data directory like--

```bash
dqlite-utils --data-dir /path/to/dqlite/data <COMMAND>
```

### Get a High-Level Status

The info command provides a snapshot of the node's Raft state, including term, index, segment counts, and a list of managed databases.

```bash
> dqlite-utils --data-dir /var/lib/dqlite info

Folder:     /var/lib/dqlite
Running:    true
Term:       2
Index:      300
VotedFor:   1
Segments:
  Closed:     2
  Open:       4
  FirstIndex: 128
  LastIndex:  300
Configuration:
  6:
    Address: 10.1.2.2
    Role: Voter
  10:
    Address: 10.1.2.3
    Role: Spare
Databases:  ["config-db", "db1", "db2"]
Snapshots:
  - Index: 128, Term: 1, Size: 19.5 KiB, Created: 2025-09-23 10:43:02
  - Index: 256, Term: 2, Size: 195.7 KiB, Created: 2025-09-23 10:44:04
```

Fields `Configuration` and `Databases` are not shown by default as it is more expensive to fetch them. It is possible to pass the `--full` flag to print them.

### Inspect the Raft Log

<!-- TODO: update this once we implemented the real output -->

For a `git log`-style history of all Raft entries, use the `log` command. This shows a chronological list of commands applied to the state machine.

```bash
> dqlite-utils --data-dir /var/lib/dqlite log

┌ 300  FRAMES database: db1, pages: 3
| 299  CONFIG
| 298    CKPT
...
| 130  FRAMES
└ 129 BARRIER ------------------------| term 2 starts
┌ 128  FRAMES database: db3, pages: 1 | term 1 ends
| 127  FRAMES database: db1, pages: 7
| 126  FRAMES database: db2, pages: 2
```

### Open a Point-in-Time Database Shell

The `open` command lets you open a **read-only** SQLite shell on a specific database at a historical point in time, specified by a Raft index. This is invaluable for understanding how data changed over time.

```bash
# Open the 'db1' database at the state it was in at index 299
> dqlite-utils --data-dir /var/lib/dqlite open db1 --index 299

db1> .tables
my_table   users
db1> SELECT COUNT(*) FROM users;
42
db1> .quit
```

## License

<!-- TODO: update this once we decided on the license -->

This project is licensed under the terms of the MIT License. See `LICENSE` file for more details.
