# snip-sync install/setup CLI hardening plan

## Context

`snip-sync` is already published as its own crate and exposes a `snip-sync` binary, but the current user path still reads like a source-tree deployment flow. The `snip-sync/README.md` starts with `cd snip-sync`, `cargo build --release`, and running `./target/release/snip-sync`. The root README links to the sync-server README but does not make the `cargo install snip-sync` path obvious. The server binary also currently has no real CLI surface: `snip-sync/src/main.rs` directly initializes tracing, checks the TLS/plaintext environment guard, ensures a current-directory config file, loads config, opens the database, and starts the gRPC/HTTP services.

The goal of this pass is to make `cargo install snip-sync` a first-class installation path and make first-run setup and service management simple enough for a user-level install. After install, the first run should create the appropriate config/state directories, create a config file in the platform-appropriate config location, and create a development certificate path using the same semantics as the existing `scripts/gen-dev-cert.sh`. The CLI should expose basic operational commands: `cert`, `edit`, `serve`, `restart`, `stop`, `update`, and `croncheck`. Documentation should include both systemd and cron supervision examples.

This is primarily a packaging, CLI, setup, and operational ergonomics pass. It should avoid expanding the sync protocol or changing the client sync semantics.

## Current repo observations

`snip-sync/Cargo.toml` defines a separate package named `snip-sync`, version `0.1.1`, and a binary named `snip-sync` at `src/main.rs`. This supports `cargo install snip-sync` as the correct published install route.

The package currently excludes `scripts/` from the published crate. That means a cargo-installed binary cannot rely on the checked-out `snip-sync/scripts/gen-dev-cert.sh` being available. Any `snip-sync cert` command must either implement generation natively in Rust or invoke an external tool directly; it must not assume the repo script is present after installation.

`Config::load()` and `Config::ensure_config_file()` currently use `CONFIG_PATH` if set and otherwise use `config.toml` in the current working directory. This is not appropriate for a cargo-installed service binary because first run from different working directories can create multiple config files. The default should become the platform config directory, with `CONFIG_PATH` retained as an override.

The database default is already effectively under `~/.config/snip-sync/snippets.db`, but it is assembled manually through `dirs::home_dir().join(".config")`. This should be normalized through a single path helper so config, certs, pid file, log/state paths, premade directory, and database defaults are consistent and testable.

The README states that production TLS should be terminated by a reverse proxy and that the server itself speaks plain gRPC. The dev certificate script prints `TLS_CERT` and `TLS_KEY`, but the current server startup does not appear to consume those environment variables; it only checks `TLS_ENABLED` or `SNIP_SYNC_ALLOW_HTTP=true`. This mismatch must be resolved in the plan implementation. For this pass, prefer treating generated certs as reverse-proxy/dev assets unless native TLS is intentionally added as a separate explicit feature. Do not document unsupported `TLS_CERT`/`TLS_KEY` behavior as if it works.

The HTTP server already exposes `/health` and returns healthy/unhealthy based on database ping. This is the correct probe target for `croncheck`.

The `snp edit` implementation already contains useful editor resolution and safety behavior. `snip-sync edit` should mirror that behavior, either by moving a minimal editor helper into a shared location or by copying a small, scoped implementation into `snip-sync`.

## Desired user experience

A user should be able to run:

```bash
cargo install snip-sync
snip-sync init
snip-sync edit
snip-sync serve
```

For a minimal local development setup, first run should be able to produce a usable config and dev certificate material without requiring a source checkout. If plaintext local serving remains required for direct `snip-sync serve`, the generated config/docs should clearly state that local direct serving uses `SNIP_SYNC_ALLOW_HTTP=true`, while the generated certs are for local reverse-proxy TLS examples unless native TLS support is implemented.

The following commands should exist:

```bash
snip-sync serve
snip-sync init
snip-sync cert
snip-sync edit
snip-sync stop
snip-sync restart
snip-sync update
snip-sync croncheck
snip-sync paths
snip-sync completions <shell>
snip-sync version
```

`serve` should also be the default command when no subcommand is provided, preserving the current behavior for existing deployments that invoke `snip-sync` directly.

`paths` is not strictly required by the original request, but it is strongly recommended. It gives users and tests a deterministic way to see the active config path, data path, cert directory, pid path, and health URL.

## Implementation sequence

### Phase 1: Add path and bootstrap primitives

Create a focused module, likely `snip-sync/src/paths.rs` or a public `snip_sync::paths` module, responsible for all default filesystem locations.

Required path helpers:

- `config_dir() -> PathBuf`: use `dirs::config_dir().unwrap_or_else(...)` and append `snip-sync`.
- `config_path() -> PathBuf`: `CONFIG_PATH` override if present, otherwise `config_dir()/config.toml`.
- `data_dir() -> PathBuf`: prefer `dirs::data_dir()/snip-sync`; fall back to config dir if unavailable.
- `state_dir() -> PathBuf`: prefer `dirs::state_dir()/snip-sync`; fall back to data dir/config dir.
- `cert_dir() -> PathBuf`: default to `config_dir()/certs` unless a better XDG-specific cert location is chosen.
- `pid_path() -> PathBuf`: default to `state_dir()/snip-sync.pid`.
- `default_db_path() -> PathBuf`: prefer `data_dir()/snippets.db`, unless preserving the existing `~/.config/snip-sync/snippets.db` default is required for backwards compatibility.
- `default_premade_dir() -> PathBuf`: prefer `data_dir()/premade-libraries` or `config_dir()/premade-libraries`; choose one and document it.

Backwards compatibility decision: because the existing default database path is under `~/.config/snip-sync/snippets.db`, changing it to `~/.local/share/snip-sync/snippets.db` could strand existing user data. Prefer preserving the current effective DB default for now, but implement it through the shared helper. Consider adding a future migration note rather than moving the DB in this pass.

Create a bootstrap function, for example `snip_sync::bootstrap::ensure_first_run_layout()`, that:

- Creates the config directory.
- Creates state/data/cert directories as needed.
- Creates the database parent directory.
- Creates the premade directory if configured/defaulted.
- Creates the default config file only if missing.
- Does not overwrite existing config/certs/database.
- Emits clear path-oriented messages for interactive commands and concise logs for `serve`.

Update `Config::load()` and `Config::ensure_config_file()` to call the new config path helper. Keep `CONFIG_PATH` as the highest-priority override. Avoid any behavior where running from a random working directory silently creates `./config.toml` unless the user explicitly set `CONFIG_PATH=./config.toml`.

Update the generated default config comments so they state the new default path and the override mechanism. Avoid stale comments saying the default config is `./config.toml`.

Acceptance criteria:

- Running `snip-sync init` from two different directories writes the same default config path unless `CONFIG_PATH` is set.
- Existing `CONFIG_PATH=/tmp/foo.toml snip-sync init` still writes `/tmp/foo.toml`.
- First-run bootstrap creates parent directories before trying to write config or database.
- Existing config files are never overwritten.
- Unit tests cover path resolution with environment overrides where feasible.

### Phase 2: Split server startup from CLI entrypoint

Refactor the current contents of `snip-sync/src/main.rs` so the server startup logic can be called from subcommands. Keep the binary thin.

Suggested structure:

- `src/main.rs`: clap parser, command dispatch, process exit handling.
- `src/server.rs`: async `serve(options: ServeOptions) -> Result<()>`, containing the current startup behavior.
- `src/bootstrap.rs`: first-run setup helpers.
- `src/cli.rs`: command enum and dispatch helpers, if keeping `main.rs` minimal.
- `src/process.rs`: PID file and lifecycle helpers.
- `src/cert.rs`: dev certificate generation.
- `src/editor.rs`: editor resolution/open helper.

Do not change sync API behavior during the refactor. Preserve the current HTTP/gRPC setup, metrics route, security headers, CORS behavior, rate limiter setup, database migration behavior, and graceful shutdown behavior.

The TLS/plaintext guard should remain enforced in `serve`. If local bootstrap wants a first-run dev mode, document and/or emit guidance rather than weakening production safety. Direct plaintext serving should still require `SNIP_SYNC_ALLOW_HTTP=true` unless the repo explicitly chooses to relax this for localhost-only binds.

Acceptance criteria:

- `cargo run -p snip-sync -- serve` starts the server with the same behavior as the previous binary.
- `cargo run -p snip-sync --` defaults to `serve` and remains backward compatible.
- Existing tests still compile.
- No sync service methods or proto-facing behavior changes in this phase.

### Phase 3: Add clap CLI surface

Add `clap` and `clap_complete` to `snip-sync/Cargo.toml` if not already available in the crate dependency graph directly. Use derive-based parsing for consistency with the root `snp` binary.

Recommended command surface:

```text
snip-sync [serve]
snip-sync init [--force-cert] [--skip-cert]
snip-sync cert [--force] [--out-dir <path>]
snip-sync edit
snip-sync stop [--force]
snip-sync restart [--force]
snip-sync update [--dry-run] [--locked]
snip-sync croncheck
snip-sync paths [--json]
snip-sync completions <shell>
snip-sync version
```

Behavior requirements:

- `serve`: run first-run bootstrap, then start server in foreground.
- `init`: run bootstrap and generate certs if missing by default. Do not start the server.
- `cert`: generate cert/key in default cert dir or explicit output dir. Refuse overwrite unless `--force` is set.
- `edit`: ensure config exists, then open it in `$EDITOR`, mirroring `snp edit` behavior.
- `stop`: stop a server process recorded by the PID file.
- `restart`: stop if running, then start in foreground or document whether it daemonizes. Prefer foreground restart for systemd and cron correctness.
- `update`: run `cargo install snip-sync` or `cargo install snip-sync --locked` depending on flag/default policy.
- `croncheck`: health-check server; if healthy, exit 0 quietly; if unhealthy/not running, start server.
- `paths`: print resolved config/data/state/cert/pid paths and health URL.
- `version`: print `snip-sync <version>`.

Be explicit that lifecycle commands are process/PID-file helpers, not a complete process supervisor. systemd remains the recommended production supervisor. croncheck is a lightweight user-level fallback.

Acceptance criteria:

- `snip-sync --help` clearly documents all commands.
- `snip-sync serve --help` and `snip-sync croncheck --help` contain enough information for headless use.
- No command requires a source checkout.
- Default `snip-sync` invocation still starts the server.

### Phase 4: Implement cert generation correctly for installed binaries

Do not shell out to `./scripts/gen-dev-cert.sh` from the installed binary. That script is excluded from the crate package and will not exist for `cargo install` users.

Preferred implementation options, in order:

1. Implement cert generation natively in Rust using an appropriate certificate-generation crate, writing `cert.pem` and `key.pem` with the same localhost SANs as the script.
2. If avoiding a new crypto/cert dependency is preferred, invoke the system `openssl` binary directly with the same arguments as the script and provide a clear error if `openssl` is unavailable.
3. Include the script in the crate package and use it only as a documented source-tree helper, not as the installed binary implementation.

The generated development cert should match current script semantics:

- Subject CN `localhost`.
- SANs: `DNS:localhost`, `IP:127.0.0.1`.
- Validity around 365 days.
- Key mode `0600` on Unix.
- Cert mode `0644` on Unix.
- Refuse to overwrite existing cert/key unless `--force` is provided.

Resolve the TLS documentation mismatch. If the server still does not support native TLS, update the script output and docs to avoid saying `TLS_CERT`/`TLS_KEY` configure snip-sync directly. Instead, phrase these as reverse-proxy/local TLS assets. If native TLS is added, add explicit config/env support and tests in a separate, clearly scoped subtask.

Acceptance criteria:

- `cargo install` users can run `snip-sync cert` successfully without a repository checkout.
- Existing cert/key files are not overwritten by default.
- File permissions are set correctly on Unix.
- Docs do not claim unsupported native TLS config.

### Phase 5: Implement `edit` with safe editor behavior

Implement `snip-sync edit` to mirror `snp edit`:

- Resolve the active config path through the shared config path helper.
- Run bootstrap/config creation if the file does not exist.
- Use `$EDITOR`, defaulting to `vim`.
- Support absolute editor paths.
- Support bare editor names found on `PATH`.
- Reject invalid absolute paths and directories.
- Handle relative paths with directory components carefully, preferably following the existing `snp edit` behavior.

Avoid introducing a dependency from `snip-sync` to the full `snip-it` crate just to share this helper unless the workspace architecture already supports that cleanly. A small duplicated helper is acceptable if it keeps the server crate independent.

Acceptance criteria:

- `EDITOR=true snip-sync edit` works in tests/CI-style environments.
- Missing config is created before invoking editor.
- Invalid editor paths return clear errors.
- Behavior is documented next to the install flow.

### Phase 6: Implement PID file and lifecycle helpers

Add process lifecycle helpers for `serve`, `stop`, `restart`, and `croncheck`.

On `serve`:

- Create the state directory.
- Acquire a lock or write PID only after confirming no valid running process is already recorded.
- If a PID file exists but is stale, remove it with a warning.
- Write the current process ID to `pid_path()`.
- Remove the PID file on graceful shutdown where possible.

On Unix `stop`:

- Read PID file.
- Validate the PID is running.
- Prefer validating command name or executable path contains `snip-sync` before sending a signal.
- Send SIGTERM.
- Wait for exit up to a bounded timeout.
- Remove stale PID files only after validation.
- Support `--force` for stale/ambiguous cases, but avoid killing arbitrary processes.

On Windows:

- Either implement a platform-specific stop path or document that `stop`/`restart` are currently Unix-first.
- If unsupported, fail clearly rather than pretending success.

`restart` should be a thin composition of stop then serve. Decide whether restart remains foreground. Prefer foreground because daemonization is best left to systemd/cron/shell. If backgrounding is implemented, document it and test it.

Acceptance criteria:

- `serve` refuses to start a duplicate instance when PID file and health check indicate an already running server.
- Stale PID files do not permanently block startup.
- `stop` does not kill unrelated processes from stale/reused PIDs.
- `restart` works for a foreground test server on Unix.

### Phase 7: Implement `croncheck`

`snip-sync croncheck` should be designed for crontab use and should not be noisy when everything is healthy.

Required behavior:

- Resolve config and run minimal bootstrap if needed.
- Construct the HTTP health URL from config: `http://<http_host>:<http_port>/health`.
- Probe health with a short timeout.
- If health returns success, exit 0 quietly.
- If health is unreachable or unhealthy, attempt to start the server.
- Use a lock file to avoid overlapping cron invocations racing into multiple starts.
- When starting from croncheck, keep behavior compatible with cron. If `serve` runs in foreground, croncheck will also remain attached; this is not desirable every five minutes. Therefore implement one of the following explicitly:
  - `croncheck` starts a detached child process running `snip-sync serve` and exits after confirming health, or
  - `croncheck` directly starts the server only for `@reboot` and docs use a separate health-only mode for every-five-minute checks.

Recommended design: `croncheck` should spawn a detached `snip-sync serve` process when recovery is required, then wait briefly for `/health` to become healthy and exit. This matches the requested crontab pattern of `@reboot` plus every five minutes.

Do not silently swallow startup failures. On failure, print a concise error and exit nonzero. On recovery, print one concise line. On healthy no-op, print nothing unless `--verbose` is added.

Acceptance criteria:

- Running `snip-sync croncheck` while the server is healthy exits quickly and does not spawn another server.
- Running it while the server is down starts exactly one server.
- Concurrent `croncheck` invocations do not start duplicate servers.
- The documented crontab works with both `@reboot` and `*/5 * * * *` entries.

### Phase 8: Implement `update`

Add `snip-sync update` as a convenience wrapper around cargo installation.

Suggested behavior:

- Locate `cargo` on `PATH`.
- Default command: `cargo install snip-sync`.
- Support `--locked` to run `cargo install snip-sync --locked` if lockfile-constrained builds are desired.
- Support `--dry-run` to print the command without executing.
- Print installed/current version before and after if practical.
- Do not attempt in-place self-replacement while the current server process is running. If running as a server, instruct the user to stop/restart or use systemd restart after update.

Acceptance criteria:

- `snip-sync update --dry-run` prints the exact cargo command.
- Missing cargo returns a clear actionable error.
- Update command does not corrupt a running service state.

### Phase 9: Documentation pass

Update `snip-sync/README.md` heavily.

Required README structure:

1. Quick install from crates.io.
2. First-run setup.
3. Local development serve.
4. Configuration path and `snip-sync edit`.
5. Dev cert generation and TLS/reverse-proxy explanation.
6. systemd example for cargo-installed binary.
7. croncheck example.
8. Source build flow for contributors.
9. Docker flow if still supported.
10. Troubleshooting.

The primary install docs should show:

```bash
cargo install snip-sync
snip-sync init
snip-sync edit
SNIP_SYNC_ALLOW_HTTP=true snip-sync serve
```

For systemd, prefer a user install example and a system install example. If using root/system service, recommend installing the binary to a stable path rather than relying on a user home cargo bin path unless the service user owns that path.

Example user-level cron docs:

```cron
@reboot /home/<user>/.cargo/bin/snip-sync croncheck
*/5 * * * * /home/<user>/.cargo/bin/snip-sync croncheck
```

Mention that croncheck is a lightweight fallback; systemd is preferred for production because it provides better logging, restart policies, and dependency management.

Update root `README.md` optional sync-server section to include at least:

```bash
cargo install snip-sync
snip-sync init
snip-sync edit
snip-sync serve
```

Retain link to the detailed `snip-sync/README.md`.

Update or remove stale documentation claiming the config default is `./config.toml`. Keep `CONFIG_PATH` as an explicit override.

Acceptance criteria:

- A user can follow the README from a clean machine with Rust installed and no source checkout.
- README does not instruct cargo-installed users to run source-tree scripts.
- README distinguishes local plaintext, reverse-proxy TLS, and production deployment accurately.

### Phase 10: Tests and validation

Add targeted tests around the new behavior.

Recommended unit tests:

- Config path defaults and `CONFIG_PATH` override.
- Bootstrap creates directories and config file without overwriting existing config.
- Cert command refuses overwrite by default.
- Editor resolver behavior copied from `snp edit` tests.
- PID stale-file detection logic.
- Health URL construction from config.
- Croncheck lock behavior.
- CLI parsing snapshots or direct parse tests for all subcommands.

Recommended integration/manual validation:

```bash
cargo check --workspace --all-targets
cargo test --workspace
cargo run -p snip-sync -- --help
cargo run -p snip-sync -- paths
cargo run -p snip-sync -- init
cargo run -p snip-sync -- cert
EDITOR=true cargo run -p snip-sync -- edit
SNIP_SYNC_ALLOW_HTTP=true cargo run -p snip-sync -- serve
cargo run -p snip-sync -- croncheck
cargo run -p snip-sync -- update --dry-run
```

If the crate publish package can be checked locally, run:

```bash
cargo package -p snip-sync --list
cargo package -p snip-sync
```

Verify that a packaged crate contains everything needed for `snip-sync init`, `snip-sync cert`, and `snip-sync serve` after installation. In particular, do not rely on files excluded by the package manifest.

Acceptance criteria:

- Workspace tests pass.
- Packaged `snip-sync` does not require a source checkout for first-run setup.
- `snip-sync` no-subcommand behavior remains backward compatible.
- Docs and behavior agree on config paths, cert purpose, and service supervision model.

## Design notes and cautions

Do not weaken the existing TLS/plaintext safety gate accidentally. The current binary refuses plaintext unless `TLS_ENABLED=true` or `SNIP_SYNC_ALLOW_HTTP=true` is present. If this behavior is changed, make it an explicit, reviewed decision. A reasonable ergonomic adjustment would be to allow plaintext automatically only for loopback binds, but that is a policy change and should be considered separately.

Be careful with PID files. A stale PID file can point to a reused PID owned by an unrelated process. `stop` must validate before sending a signal. Prefer health checks and executable-name validation over blind `kill`.

Be careful with cron semantics. A command that starts the server in foreground is not appropriate for a five-minute cron entry unless cron’s process lifecycle is intentionally used as the supervisor. The requested `croncheck` should therefore be implemented as a lightweight supervisor check that spawns or delegates to a detached server process only when needed, then exits.

Be careful with cert docs. The existing script currently prints `TLS_CERT` and `TLS_KEY`, but the server does not consume those variables. Either remove that implication or implement native TLS explicitly. Do not leave users with config knobs that look supported but do nothing.

Avoid introducing a dependency cycle between `snip-it` and `snip-sync`. Shared utility extraction is fine only if it remains clean. Duplicating a small editor resolver in `snip-sync` is acceptable for this pass.

## Suggested file changes

Likely modified files:

- `snip-sync/Cargo.toml`
- `snip-sync/src/main.rs`
- `snip-sync/src/lib.rs`
- `snip-sync/src/server.rs`
- `snip-sync/src/paths.rs`
- `snip-sync/src/bootstrap.rs`
- `snip-sync/src/cert.rs`
- `snip-sync/src/editor.rs`
- `snip-sync/src/process.rs`
- `snip-sync/config.toml`
- `snip-sync/config.example.toml`
- `snip-sync/scripts/gen-dev-cert.sh`
- `snip-sync/README.md`
- `README.md`

Potential new tests:

- `snip-sync/tests/cli.rs`
- `snip-sync/tests/bootstrap.rs`
- `snip-sync/tests/process.rs`

## Final acceptance checklist

- `cargo install snip-sync` is documented as the primary install path.
- First run creates a stable config file in the platform config location, not in arbitrary current working directories.
- First run creates required directories and dev certs without requiring a source checkout.
- `snip-sync edit` opens the active config file with `$EDITOR` and creates it if missing.
- `snip-sync serve` remains foreground and systemd-friendly.
- `snip-sync stop` and `snip-sync restart` work safely where supported.
- `snip-sync update --dry-run` and `snip-sync update` provide a clean cargo-based update path.
- `snip-sync croncheck` can be used safely from `@reboot` and every-five-minute crontab entries without spawning duplicates.
- systemd and cron examples are both present in `snip-sync/README.md`.
- Documentation no longer implies unsupported native TLS configuration.
- Workspace checks/tests pass.
