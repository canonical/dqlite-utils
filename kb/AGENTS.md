# Preface

This is the root index file of the dispersed knowledge base (`kb/`). It is the main entry point for navigating the repository, understanding subsystem boundaries, and maintaining agent-oriented documentation conventions.

# Overview

This repository keeps agent guidance in local `kb/AGENTS.md` files near the code they describe. Read this file first, then load only the local KB files for the directories you are actively working in.

The structure is designed to stay:

- Mechanical: agents are expected to read and maintain these files.
- Distilled: avoid verbose task logs and duplicated architecture notes.
- Hierarchical: keep context local instead of centralizing everything in one file.
- Human-readable: short, direct notes are preferred over opaque reminders.

# Important

- Read local `kb/AGENTS.md` files whenever you enter a relevant directory.
- Update the nearest KB files when code structure, workflow, or sharp edges change in a way future agents should know.
- Keep KB notes specific to agent navigation, boundaries, recurring pitfalls, and repository workflow. Do not turn KB files into generic changelogs.
- Follow the header conventions below. `Preface` is required in every KB markdown file; other headers are optional and should only be used when they add real signal.

# Headers

Every header used in `kb/*.md` files must be documented here.

- `Preface`: Briefly defines the scope and purpose of the local KB file. Required at the top of every KB markdown file.
- `Overview`: High-level summary of a directory, subsystem, or workflow. Do not use it as a file listing.
- `Important`: Critical constraints, workflow rules, or recurring failure modes.
- `Headers`: Global registry of allowed KB headers. Only the root `kb/AGENTS.md` should contain this section.
- `Architecture`: Structural boundaries, lifecycle rules, or design constraints for a subsystem. Use this for software architecture, not for directory listings.
- `Directory`: Describes the contents of the current directory. Format entries as ``- `name` - Description``.
- `Index`: Lists local KB files or child KB entry points. This section must appear last when present. Format entries as ``- `path` - Description``.

# Index

- `../src/kb/AGENTS.md` - Source tree layout and Rust-specific workflow
- `../src/command/kb/AGENTS.md` - REPL command parsing, shells, and command-authoring rules
- `../src/dqlite/kb/AGENTS.md` - Raft and snapshot decoding, FFI boundaries, and safety rules
- `../src/rusqlite_ext/kb/AGENTS.md` - SQLite wrappers, VFS support, and extension-layer constraints
- `../snap/kb/AGENTS.md` - Snap packaging metadata and build dependencies
- `../.github/kb/AGENTS.md` - CI workflows and repository automation
