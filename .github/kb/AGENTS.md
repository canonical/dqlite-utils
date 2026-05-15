# Preface

This file documents repository automation under `.github/`, with emphasis on workflows and composite actions that enforce the project’s Rust quality gates.

# Overview

GitHub Actions runs a single build-and-test workflow on pushes to `main` and on pull requests. The workflow installs the pinned Rust toolchain, provisions native dqlite dependencies, runs lint checks, and executes tests in both debug and release profiles.

# Important

- CI currently enforces `cargo fmt --all --check`, `cargo check --locked`, `cargo check --locked --all-features`, `cargo clippy --locked --all-features --all-targets --tests`, `cargo doc --no-deps`, and `cargo test` in debug and release.
- Workflow changes that alter required commands or environment assumptions should be mirrored in the relevant KB files so future agents do not rely on stale local workflows.

# Directory

- `workflows/build-and-test.yml` - Main CI workflow for linting, building, and testing.
- `actions/cargo-lint/action.yml` - Composite action for format, check, clippy, and docs steps.
- `actions/cargo-test/action.yml` - Composite action for debug and release test execution.
