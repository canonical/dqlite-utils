# `dqlite-utils`

`dqlite-utils` is an observability tool for inspecting the on-disk state of a `dqlite` node.
The inspected dqlite node does not have to be stopped and does not have to be the leader allowing issues to be debugged without interacting with a live process or the rest of the cluster.

Thus, `dqlite-utils` never affects existing data or server execution.

## Installation

### From snapcraft.io

To use the latest release, install from `snapcraft.io`:

```bash
sudo snap install dqlite-utils
```

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

This tool opens a REPL to query the content of a dqlite folder.
By default, it works on the current folder, however the path to the dqlite data directory can be specified:

```bash
dqlite-utils --dir /path/to/dqlite/data
>
```

To use `dqlite-utils` non-interactively, pass commands via the `-c` flag.

To see available commands, type and run `.help`.
For help on a specific command, type and run `.help .name-of-command`.

### Inspect the Node Status

The `.status` command shows a brief summary the current state of the Raft state machine.

```bash
> .status
dir: .
term: 1
current_index: 20011
first_index: 10242
```

### Inspect the Raft Configuration

The `.config` command prints the current Raft configuration.

```bash
> .config
- ID: 1
  Address: 10.0.0.1:9001
  Role: voter
- ID: 2
  Address: 10.0.0.2:9001
  Role: standby
```

Use `--raw` to print numeric role values:

```bash
> .config --raw
- ID: 1
  Address: 10.0.0.1:9001
  Role: 1
- ID: 2
  Address: 10.0.0.2:9001
  Role: 0
```

### Inspect the Raft Log

To view the log, use the `.log` command.
You will see a list of all commands applied to the dqlite state machine, for example:

```bash
> .log
╭ TERM 3
│ 300 COMMIT db1
│   tx_id: 12345
│   truncate: 0
│   pages: 1, 2, 3
│ 299 CONFIG
│   servers:
│     1:
│       address: 10.0.0.1:9001
│       role: Voter
│ 298 CHECKPOINT db1
├ TERM 2
│ 297 COMMIT db2
│   tx_id: 12340
│   truncate: 0
│   pages: 5, 6
│ 296 BARRIER
```

Use the `--compact` flag to show a condensed view without detailed information:

```bash
> .log --compact
╭ TERM 3
│ 300 COMMIT db1
│ 299 CONFIG
│ 298 CHECKPOINT db1
├ TERM 2
│ 297 COMMIT db2
│ 296 BARRIER
```

### Create a snapshot

To create a snapshot to load into a dqlite cluster, use the `.snapshot` shell.
For example, to create a new snapshot in `snapshot-dir/` which contains a single server and includes some `existing.db`, type and run the following commands.

```bash
.snapshot
snapshot> .add-server 237.54.182.68:12345
snapshot> ATTACH DATABASE "existing.db" AS existing;
snapshot> .finish snapshot-dir
```

You should see a snapshot inside of `snapshot-dir` and that the REPL has returned to its default shell.

When creating a snapshot, to edit Raft metadata, use the `.set-index`, `.set-term` and `.set-timestamp` commands.
To see the current state of the snapshot being created, type and run `.info`.
You will see output like this:

```bash
term: 1
index: 1
timestamp: 2026-01-20T13:35:26.000000000Z
configuration:
- id: 1
  address: 237.54.182.68:12345
  role: voter
```

To abort snapshot creation, type and run the `.abort` command.
You will see the REPL’s default shell.
