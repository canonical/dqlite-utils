# Preface

This file describes the `src/command/` subtree, which owns REPL command parsing, shell transitions, and help registration.

# Overview

Commands are parsed into the `Command` enum in `src/command/mod.rs`. Dot-prefixed input is resolved against the shared command namespace, while non-dot input is treated as SQL. Availability is enforced at run time based on the active shell.

# Important

- New commands must be wired into the relevant command enum, `CommandKind`, and shell help output together; help coverage is part of the maintained behavior.
- Shell transitions happen by replacing `ctx.shell`. Nested shell commands must leave the context in a valid shell state when they exit.
- SQL execution is only valid in shells that own an attached SQLite connection, currently the snapshot and open shells.

# Architecture

The root shell exposes high-level read-only inspection commands such as `.status`, `.config`, and `.log`, plus entry points into nested shells like `.snapshot` and `.open`. `snapshot/` owns synthetic snapshot creation and metadata editing. `open/` owns loading raft-backed database state into SQLite through the custom VFS layer. `sql/` handles plain SQL statements after they have been separated from dot commands.

# Directory

- `mod.rs` - Command enums, parsing, dispatch, and shell command registration.
- `help.rs` - Shared help rendering and tests that assert command coverage.
- `config.rs` - Raft configuration inspection.
- `status.rs` - High-level node status output.
- `log.rs` - Raft log inspection and formatting.
- `quit.rs` - REPL exit behavior.
- `snapshot/` - Snapshot shell commands such as `.add-server`, `.set-index`, and `.finish`.
- `open/` - Open shell commands and database browsing helpers.
- `sql/` - SQL command parsing and execution in shells with a connection.
