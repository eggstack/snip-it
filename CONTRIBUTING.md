# Contributing to snip-it

Thank you for your interest in contributing to snip-it! This document
provides guidelines and information for contributors.

## Development Setup

### Prerequisites

- **Rust:** Latest two stable releases (currently 1.94+). A
  `rust-toolchain.toml` is checked in to pin the local toolchain.
- **Protobuf compiler (`protoc`)** — required *only* if you regenerate
  the gRPC stubs. The generated `snip-proto/src/snip_proto.rs` is
  committed, and `src/proto.rs` is inlined into the binary, so a
  plain `cargo build` does not need `protoc`.
- **OpenSSL headers** (Linux) — required by `tonic`'s TLS backend.

### Getting Started

```bash
git clone https://github.com/eggstack/snip-it.git
cd snip-it
cargo build --release
cargo test
```

### Development Environment

```bash
# Run snp locally (development build)
cargo run -- run

# Run the sync server locally for testing
cd snip-sync
SNIP_SYNC_ALLOW_HTTP=true cargo run

# In another terminal, register with the local server
cd ..
cargo run -- register http://localhost:50051

# Test sync operations
cargo run -- sync
```

## Development Workflow

### Code Style

- Follow existing code conventions — mimic surrounding code style.
- Run `cargo fmt` before committing.
- Run `cargo clippy --all-targets -- -D warnings` to check for lint
  issues. CI fails on any clippy warning.
- Do not add comments unless asked.
- Keep lines under 100 characters (enforced by `rustfmt.toml`).
- Prefer `?` over `.unwrap()` for error propagation in non-test code.

### Testing

```bash
# Focused developer verification (same as CI runs on Linux)
bash scripts/check.sh

# Run all tests including deep integration suites
cargo test --workspace --all-features -- --test-threads=1

# Run server tests
cargo test -p snip-sync
```

CI runs `scripts/check.sh` on Linux and smoke tests on macOS/Windows.
Deep crash, restore, and protocol suites run in `release-check.sh` before
releases, not on every push.

### Error Handling

- Use `SnipError` for all error types (`src/error.rs`).
- Use convenience constructors: `SnipError::io_error()`,
  `SnipError::toml_error()`, etc.
- Return `SnipResult<T>` from functions that can fail.
- Prefer `?` over `.unwrap()` for error propagation.

### Commit Messages

- Use imperative mood ("Add feature" not "Added feature").
- Keep the first line under 72 characters.
- Reference issues / PRs when applicable (`Refs #42`, `Fixes #17`).
- One logical change per commit.

### Branching

- `main` is the release branch; keep it green.
- Feature work happens on topic branches; squash-merge into `main`.
- Long-lived branches: `release/v1.x` for maintenance.

## Project Structure

```
snip-it/
├── Cargo.toml          # Main binary crate (`snp`, published to crates.io)
├── src/                # CLI source
├── snip-proto/         # Protobuf + tonic stubs (published to crates.io)
├── snip-sync/          # Sync server binary (published to crates.io)
├── tests/              # Integration tests
├── assets/             # vhs tape for the demo GIF (rendered to demo/)
├── architecture/       # Internal architecture docs (AI-agent oriented)
├── .skills/            # AI-agent context files
├── AGENTS.md           # AI-agent contributor guide
└── AGENTS.override.md  # Per-user overrides for AI-agent context
```

The `architecture/`, `.skills/`, `AGENTS.md`, and `AGENTS.override.md`
files are **kept in the public repo on purpose** so
that contributors using AI coding agents have the same context the
maintainer does. They are excluded from the published crate.

## Pull Requests

- Describe what changed and why in the PR body.
- Reference any related issue.
- Make sure CI is green.
- New features should include a test where practical.
- Bug fixes should include a regression test.

## Reporting Issues

Please use the GitHub issue tracker to report bugs or request features.
Include:

- Operating system and version
- Rust version (`rustc --version`)
- `snp` version (`snp version`)
- Steps to reproduce
- Expected vs actual behavior
- Relevant log output from `~/.config/snp/logs/`

**For security issues, do not open a public issue.** Follow
[SECURITY.md](SECURITY.md) instead.

## Release Process

Publishing to crates.io is a **manual, local** operation. There is no
automated publish workflow and no crates.io token in GitHub Actions.

1. **Bump version** in the changed crate's `Cargo.toml`. If `snip-proto`
   changed, update its dependency version in `snip-sync` and `snip-it`.
2. **Update `CHANGELOG.md`**: move `[Unreleased]` entries under a new
   dated version heading; add a link reference at the bottom.
3. **Open a PR** with both changes; get review and merge.
4. **Run release checks** locally from a clean checkout:
   ```bash
   bash scripts/release-check.sh verify
   ```
5. **Dry-run each changed crate** in dependency order:
   ```bash
   bash scripts/release-check.sh dry-run snip-proto   # only if changed
   bash scripts/release-check.sh dry-run snip-sync    # only if changed
   bash scripts/release-check.sh dry-run snip-it
   ```
6. **Publish** manually in dependency order (see [RELEASING.md](RELEASING.md)):
   ```bash
   cargo publish -p snip-proto   # only if snip-proto changed
   cargo publish -p snip-sync    # only if snip-sync changed
   cargo publish -p snip-it
   ```
7. **Tag** (optional):
   ```bash
   git tag -a vX.Y.Z -m "snip-it vX.Y.Z"
   git push origin vX.Y.Z
   ```
8. **Verify** from a clean machine:
   ```bash
   cargo install snip-it --version X.Y.Z --locked
   snp --version
   ```

## MSRV Policy

The `rust-version` field in `Cargo.toml` is the minimum supported Rust
version. We support the **latest two stable Rust releases**. Bumping
the MSRV is **not** a breaking change and will not trigger a major
version bump; it will be called out in `CHANGELOG.md`. A release that
intentionally drops support for an older toolchain is the exception
and will be documented.

## License

By contributing, you agree that your contributions will be licensed
under the MIT License.
