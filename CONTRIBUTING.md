# Contributing to snip-it

Thank you for your interest in contributing to snip-it! This document provides guidelines and information for contributors.

## Development Setup

### Prerequisites

- Rust 1.88 or later
- Protobuf compiler (`protoc`) — required for building `snip-proto`

### Getting Started

```bash
# Clone the repository
git clone https://github.com/anomalyco/snip-it.git
cd snip-it

# Build the project
cargo build --release

# Run all tests
cargo test
```

### Development Environment

```bash
# Run snp locally (development build)
cargo run -- run

# Run the sync server locally for testing
cd snip-sync
cargo run

# In another terminal, register with the local server
cargo run -- register http://localhost:50051

# Test sync operations
cargo run -- sync
```

## Development Workflow

### Code Style

- Follow existing code conventions — mimic surrounding code style
- Run `cargo fmt` before committing
- Run `cargo clippy --all-targets -- -D warnings` to check for lint issues
- Do not add comments unless asked
- Keep lines under 100 characters

### Testing

```bash
# Run all tests (unit + integration)
cargo test

# Run only integration tests
cargo test --test integration

# Run server tests
cargo test -p snip-sync

# Format check
cargo fmt --check

# Lint
cargo clippy --all-targets -- -D warnings
```

### Error Handling

- Use `SnipError` for all error types (`src/error.rs`)
- Use convenience constructors: `SnipError::io_error()`, `SnipError::toml_error()`, etc.
- Return `SnipResult<T>` from functions that can fail
- Prefer `?` operator over `.unwrap()` for error propagation

### Commit Messages

- Use imperative mood ("Add feature" not "Added feature")
- Keep first line under 72 characters
- Reference issues when applicable

## Project Structure

- `src/` — Main CLI binary (`snp`)
- `snip-sync/` — Sync server (gRPC + HTTP)
- `snip-proto/` — Protobuf definitions (shared)
- `tests/` — Integration tests
- `architecture/` — Architecture documentation
- `.skills/` — AI-agent context files

## Reporting Issues

Please use the GitHub issue tracker to report bugs or request features. Include:

- Operating system and version
- Rust version (`rustc --version`)
- Steps to reproduce
- Expected vs actual behavior

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
