# Self-Update Transport

[← Back to Overview](overview.md)

## Overview

`snp update` and `snip-sync update` share the same binary-first strategy —
crates.io selects the stable version, the exact component GitHub tag supplies
the binary + checksum, and the installed executable changes only after the
candidate passes integrity and identity checks — but they use deliberately
different transports. The **client** fetches in-process via
`eggfetch-core`; the **server** shells out to external `curl`. That split is a
measured footprint tradeoff, not an oversight.

This doc covers the update transports and the install/replace machinery. It
does **not** cover the release/publish process (see
[`RELEASING.md`](../RELEASING.md) and [`CONTRIBUTING.md`](../CONTRIBUTING.md)),
the crate-version-to-tag contract beyond what the code pins, the server's HTTP
leaf (see [server.md](server.md)), or the update server's gRPC surface.

**Sources**: `src/update.rs` (1769 lines), `snip-sync/src/update.rs` (778),
`src/main.rs:70-83` (CLI flags), `src/main.rs:451-465` (dispatch),
`tests/architecture.rs:163-256` (source-level pins).

## Module Map

| File | Role |
|------|------|
| `src/update.rs` | Client updater: version check, asset download, checksum + identity verification, executable replacement |
| `snip-sync/src/update.rs` | Server updater: same strategy over external `curl`, with lifecycle-aware stop/restart |
| `src/main.rs:70-83` | `snp update --dry-run/--locked` and the hidden `__self-replace` subcommand |
| `snip-sync/src/cli.rs:50-68` | `snip-sync update --dry-run/--locked` and the hidden `SelfReplace` subcommand |
| `tests/architecture.rs:163-256` | Pins that lock the two-transport split at source level |

## The Two-Transport Split

| | `snp update` (client) | `snip-sync update` (server) |
|---|---|---|
| Transport | in-process `eggfetch-core` | external `curl` subprocess |
| Dependency | `eggfetch-core = { version = "=0.2.2", default-features = false, features = ["standard-http1", "redirects", "tls-rustls", "tls-native-roots"] }` (`Cargo.toml:132-137`) | none — `curl` must be on `PATH` |
| Redirect handling | `RedirectPolicy::strict(10)` inside eggfetch | `curl --location` (curl's own default redirect cap) |
| Overall deadline | native per-request `Timeout.total` (60 s) | `curl --max-time 60` |
| Initial-URL scheme | HTTPS only in production (`build_allow_http()` is `test-support` only) | HTTPS only in production (`validate_https_url`; `test-helpers` widens it) |
| Tag / asset | `v{version}` / `snp-{target}[.exe]` (`src/update.rs:535`, `:539`) | `snip-sync-v{version}` / `snip-sync-{target}[.exe]` (`snip-sync/src/update.rs:316`, `:320`) |

### Why the server stayed on `curl`

The server's own module header records the measurement
(`snip-sync/src/update.rs:7-13`):

> Plan 017's controlled same-toolchain lean `eggfetch-core 0.2.0` trial grew
> this deliberately small server binary from 3,833,152 to 5,145,224 bytes
> (**+34.23%**), past the plan's 10% material-growth gate. The server remains
> on curl; do not consolidate this adapter without re-running the measurement
> and preserving equivalent updater behavior.

`snip-sync` is deliberately a small binary, so linking the HTTP/TLS stack into
it costs far more than the client, where the same dependency is amortized
across a much larger binary. **Do not unify the two transports** without
re-running the size measurement and preserving equivalent behavior (checksum
contract, identity probe, lifecycle-aware restart).

## `snp update`: In-Process eggfetch-core

**File**: `src/update.rs`

### Entry Point and Flags

```rust
// src/update.rs:80
pub async fn run(dry_run: bool, _locked: bool) -> Result<(), String>
```

CLI (`src/main.rs:70-75`):

```rust
Update {
    #[arg(long, help = "Check for an update without installing it")]
    dry_run: bool,
    #[arg(long, help = "Use Cargo's locked dependency versions")]
    locked: bool,
},
```

Dispatched through the global runtime: `RUNTIME.block_on(update::run(dry_run, locked))`
(`src/main.rs:451-457`). `snp update` is classified
`SuppressConfiguration` + `Logging` (`src/main.rs:981-991`).

- **`--dry-run`** stops before any download or write: `run()` prints the plan
  and returns `Ok(())` (`src/update.rs:93-96`), via `print_dry_run()`
  (`src/update.rs:145`). For a prebuilt host it names the exact asset and
  exact tag that *would* be fetched; for a source-only or unsupported host it
  says the Cargo fallback would run. Homebrew-managed installs print the
  `brew upgrade` command that would run. Nothing is downloaded, no file is
  written, and no subprocess is spawned.
- **`--locked` is accepted but currently ignored.** The parameter is bound as
  `_locked` (`src/update.rs:80`), so the flag has no effect on the client's
  behavior. It is not a no-op in practice: the Cargo fallback **always** passes
  `--locked` to `cargo install` regardless of the flag
  (`src/update.rs:641-650`), so the resolved dependency graph is always the
  locked one. The same is true of the server
  (`snip-sync/src/update.rs:54`, `pub fn run(dry_run: bool, _locked: bool)`;
  its always-locked `cargo install` is at `snip-sync/src/update.rs:417-423`).

### Version Resolution

- The current version is `CARGO_PKG_VERSION` parsed as semver
  (`current_version`, `src/update.rs:141`).
- The candidate version is crates.io `max_version`
  (`latest_crates_version`, `src/update.rs:238`), fetched from
  `https://crates.io/api/v1/crates/{crate}` (`src/update.rs:20`).
- Prerelease versions are rejected outright (`src/update.rs:250-255`).
- `latest <= current` prints "already up to date" and exits without writing
  (`src/update.rs:87-90`).

### Install Method Detection

`detect_install_method()` (`src/update.rs:181`) classifies the canonicalized
running executable:

| Method | Detection | Update path |
|--------|-----------|-------------|
| `Homebrew` | the executable starts with `brew --prefix snip-it` (`homebrew_formula_prefix`, `src/update.rs:221`) | `brew upgrade snip-it` (`update_with_homebrew`, `src/update.rs:907`) — the binary is never touched by `snp` |
| `Cargo` | the executable lives under `$CARGO_HOME/bin` (or `~/.cargo/bin`), or under a `bin/` whose parent holds `.crates.toml` / `.crates2.json` (`is_cargo_install_path`, `src/update.rs:206`) | binary-first: a prebuilt asset is preferred; Cargo is the fallback |
| `Direct` | everything else | binary-first: a prebuilt asset is preferred; Cargo is the fallback |

Homebrew is checked **first** — a Homebrew-managed binary is replaced by
`brew`, never overwritten in place (`src/update.rs:98-100`).

### Target Mapping and Asset Names

`host_target()` (`src/update.rs:522`) maps OS/arch to a release target:

| OS / arch | Target | Kind |
|-----------|--------|------|
| linux / x86_64 | `x86_64-unknown-linux-gnu` | prebuilt |
| linux / aarch64 | `aarch64-unknown-linux-gnu` | prebuilt |
| linux / arm | `armv7-unknown-linux-gnueabihf` | source-only → Cargo |
| macos / x86_64 | `x86_64-apple-darwin` | prebuilt |
| macos / aarch64 | `aarch64-apple-darwin` | prebuilt |
| windows / x86_64 | `x86_64-pc-windows-msvc` | prebuilt |
| windows / aarch64 | `aarch64-pc-windows-msvc` | source-only → Cargo |
| anything else | — | unsupported → Cargo |

Asset name: `snp-{target}[.exe]` (`asset_name`, `src/update.rs:539`; `.exe` only
on Windows). Tag: `v{version}` (`component_tag`, `src/update.rs:535`). The
download URL is
`{release_base}/v{version}/snp-{target}[.exe]` with a `.sha256` sidecar
(`download_candidate`, `src/update.rs:550-581`).

### Transport Guarantees

```rust
// src/update.rs:333-349
fn update_http_client() -> eggfetch_core::Client {
    http_client_with(FETCH_CONNECT_TIMEOUT, FETCH_READ_TIMEOUT)
}

fn http_client_with(connect: Duration, read: Duration) -> eggfetch_core::Client {
    eggfetch_core::Client::builder()
        .user_agent("snip-it-update")
        .automatic_decompression(false)
        .redirect_policy(update_redirect_policy())
        .timeout(
            eggfetch_core::Timeout::builder()
                .connect(connect)
                .read(read)
                .build(),
        )
        .build()
}
```

- **Lean feature profile** (`Cargo.toml:132-137`): `default-features = false`
  plus exactly `standard-http1`, `redirects`, `tls-rustls`,
  `tls-native-roots` — the lean HTTP/1.1 route with native-root TLS. No broad
  `http1` alias.
- **No automatic decompression** (`.automatic_decompression(false)`): release
  bytes must stay byte-for-byte so the SHA-256 sidecar verifies the artifact
  actually shipped.
- **Strict redirects, delegated** (`update_redirect_policy`,
  `src/update.rs:323`): `eggfetch_core::RedirectPolicy::strict(MAX_REDIRECTS)`
  with `MAX_REDIRECTS = 10` (`src/update.rs:279`) — bounded depth plus
  HTTPS→HTTP downgrade denial. One dispatch (`fetch_response`,
  `src/update.rs:358`) validates the initial scheme, then sends a single GET.
  Redirect traversal is eggfetch's job; there is no manual redirect loop.
- **Native total timeout** (`FETCH_OVERALL_TIMEOUT = 60 s`,
  `src/update.rs:287`): applied per request via
  `eggfetch_core::Timeout::builder().total(overall_timeout)`
  (`src/update.rs:370-374`). It spans redirect traversal through final-body
  EOF and does **not** reset per hop or per chunk — this is the parity point
  with the former `curl --max-time 60`. Client-level connect (10 s,
  `src/update.rs:288`) and read (60 s, `src/update.rs:289`) defaults are
  retained as per-field overrides.
- **Initial-HTTPS guard** (`check_url_scheme`, `src/update.rs:305` /
  `scheme_allowed`, `src/update.rs:298`): a non-HTTPS initial URL is rejected
  before any I/O. `build_allow_http()` (`src/update.rs:294`) is
  `cfg!(feature = "test-support")` — **production builds are HTTPS-only**, and
  the widening exists solely so the loopback fixtures can be injected.
- **No retries**: the updater must not hide transport or release defects.
- **Size caps**: metadata ≤ 1 MiB (`MAX_METADATA_BYTES`, `src/update.rs:22`),
  binary ≤ 256 MiB (`MAX_BINARY_BYTES`, `src/update.rs:23`), enforced by
  `max_decoded_body_size` (`src/update.rs:369`).
- **404 is not an error**: it maps to `FetchError::NotFound`
  (`src/update.rs:430`, `:469`), which for a release asset means "no prebuilt
  asset for this target" → Cargo fallback (`src/update.rs:110-116`).

### Download, Verification, Replacement

- **Streaming**: the release asset is written chunk-by-chunk to a staging file
  without buffering the whole binary in memory (`stream_binary_to_file`,
  `src/update.rs:502`). A failed transfer removes the partial staging file
  (`remove_partial_staging_file`, `src/update.rs:495`) so a partial candidate
  can never be installed.
- **Checksum**: the `.sha256` sidecar must be exactly one line of
  `<64-hex-digest>  <asset-name>`; anything else is rejected
  (`verify_checksum`, `src/update.rs:583-624`). The digest comparison is
  case-insensitive.
- **Identity probe**: the candidate is executed with `version` and must print
  exactly `snp {version}` (`validate_candidate`, `src/update.rs:679-704`), with
  a 10 s timeout and a kill on expiry (`wait_for_candidate`,
  `src/update.rs:706`).
- **Write probe**: `ensure_destination_writable()` (`src/update.rs:735`)
  creates and removes a uniquely named probe file in the destination's parent
  directory **before** any download, so an unwritable install fails fast
  instead of after a multi-megabyte transfer.
- **Unix replacement** (`replace_installed_executable`, `src/update.rs:768`):
  copy the candidate beside the destination, copy the destination's
  permissions, `sync_all()`, then `rename()` over the destination — atomic on
  the same filesystem. Every failure path removes the staged file.
- **Windows replacement** (`schedule_windows_self_replace`,
  `src/update.rs:838`): the running executable is copied to a helper binary
  and spawned as `__self-replace --candidate … --destination …`
  (`src/update.rs:850-859`); the helper performs `MoveFileExW` with
  `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`
  (`run_self_replace_helper`, `src/update.rs:868`). The hidden subcommand is
  declared at `src/main.rs:76-83` and dispatched at `src/main.rs:458-465`; it
  is a no-op error on non-Windows.

### Cargo Fallback

`cargo_candidate()` (`src/update.rs:630`) runs:

```text
cargo install <crate> --version =<version> --locked --root <staging>/cargo-root
```

and then requires `<staging>/cargo-root/bin/snp` to exist
(`src/update.rs:664-675`). The fallback is used for source-only hosts,
unsupported hosts, and a definite missing (404) release asset — never for a
transient download failure. `--locked` is always passed here, independent of
the CLI `--locked` flag.

## `snip-sync update`: External `curl`

**File**: `snip-sync/src/update.rs`

- `run(dry_run, _locked)` (`snip-sync/src/update.rs:54`) — same overall
  strategy, but it first resolves the **service lifecycle**
  (`snip_sync::startup::update_lifecycle()`), because the server may be
  running as a managed service and must be stopped before the binary is
  replaced, then restarted afterwards (`restart_after_update`,
  `snip-sync/src/update.rs:109`).
- `--dry-run` prints the target/tag that would be used plus the resolved
  lifecycle state and states that "no files, processes, or services were
  changed" (`print_dry_run`, `snip-sync/src/update.rs:121-137`).
- **The `curl` adapter**: `fetch_bytes` (`snip-sync/src/update.rs:186`) and
  `fetch_file` (`snip-sync/src/update.rs:232`) invoke
  `curl --silent --show-error --location --proto =https --tlsv1.2
  --connect-timeout 10 --max-time 60 --max-filesize <cap> --user-agent
  snip-it-update`, then read the HTTP status from `--write-out`.
- `--proto` is `=https` in production and `=http,https` under the
  `test-helpers` feature (`curl_protocol`, `snip-sync/src/update.rs:175-184`).
- `validate_https_url()` (`snip-sync/src/update.rs:162`) rejects non-HTTPS
  URLs before the subprocess runs; the `test-helpers` build accepts loopback
  `http://`.
- The server has its own hidden `SelfReplace` subcommand
  (`snip-sync/src/cli.rs:57-68`, dispatched at `snip-sync/src/main.rs:35`),
  which additionally accepts `--restart` and `--managed-restart`
  (`snip-sync/src/update.rs:632-637`).

## Architecture Pins

`tests/architecture.rs` locks the split at source level:

| Test | Pin |
|------|-----|
| `snp_updater_does_not_shell_out_to_curl` (`:172`) | `src/update.rs` contains no `"curl"` / `curl_protocol` in non-comment lines (the server file is explicitly excluded from the scan) |
| `snp_updater_pins_lean_eggfetch_profile` (`:201`) | `Cargo.toml` contains `eggfetch-core = { version = "=0.2.2"` plus `"standard-http1"` and `"redirects"` |
| `snp_updater_delegates_redirects_and_total_timeout` (`:221`) | `src/update.rs` contains `RedirectPolicy::strict` and contains no `tokio::time::timeout`, `fn safe_get`, `fn is_redirect_status`, `fn redirect_location`, `fn redirect_target`, `follow_redirects(false)` |

> **Resolved.** The dependabot bump `bd8e988` raised the pin in
> `Cargo.toml:132` from `=0.2.0` to `=0.2.2`, which left
> `tests/architecture.rs:207` asserting a literal string that no longer
> matched the manifest — `snp_updater_pins_lean_eggfetch_profile` failed on
> `main`. The assertion now pins `=0.2.2`, matching the manifest. **Any
> future bump of this pin must update the test string in the same change**,
> or the architecture suite goes red again.

There is **no** source-level pin that keeps the *server* on `curl` — the
no-curl test deliberately skips `snip-sync/src/update.rs`, and the server's
`curl` usage is guarded only by the module comment and this doc.

## Transport Tests

Updater transport tests live in `src/update.rs` under the `test-support`
feature (`mod transport_tests`, `src/update.rs:1035-1036`) with a **std-only
loopback fixture** — a hand-written single-threaded HTTP/1.1 server on
`TcpListener::bind("127.0.0.1:0")` (`src/update.rs:1130`). No mock framework,
no second HTTP client, no Tokio test runtime, no external network.

- The fixture routes on the request path, counts every hit (so a test can
  prove a target was **never** contacted), and can slow-drip a body in chunks
  to prove the overall deadline fires while body progress is underway
  (`Fixture`, `src/update.rs:1117-1189`).
- Production-policy cases pass `allow_http = false` explicitly, so the
  HTTPS-only rule is proven against live fixtures without TLS test
  certificates (`initial_scheme_gate_still_owns_production_https_only`,
  `src/update.rs:1669-1676`).
- `redirect_policy_is_strict` (`src/update.rs:1648-1666`) asserts the
  updater *selects* the strict policy (`follow`, `max_redirects ==
  MAX_REDIRECTS`, `downgrade == RedirectDowngradePolicy::Deny`) rather than
  re-testing eggfetch's own redirect engine. The comment there notes that
  downgrade rejection itself is qualified by eggfetch 0.2.2 behavior — the
  pinned dependency is `=0.2.2` (`Cargo.toml:132`), matching the version
  asserted by `tests/architecture.rs`. The qualification should be
  re-confirmed whenever the pin moves.
- Endpoint overrides (`SNIP_UPDATE_CRATES_API_URL`,
  `SNIP_UPDATE_RELEASE_BASE_URL`) exist only under `test-support` /
  `test-helpers` (`src/update.rs:259-269`, `snip-sync/src/update.rs:150-160`)
  and are compiled out of production builds.

## Key Files

- `src/update.rs` — client updater: transport, install-method detection, download, verification, replacement
- `src/update.rs:1035` — `test-support` transport tests with the std-only loopback fixture
- `snip-sync/src/update.rs` — server updater: `curl` adapter, lifecycle-aware stop/restart, size-gate rationale
- `snip-sync/src/cli.rs:50-68` — server `update` flags and the hidden `SelfReplace` subcommand
- `src/main.rs:70-83` — client `update` flags and the hidden `__self-replace` subcommand
- `Cargo.toml:132-137` — the lean `eggfetch-core` feature set
- `tests/architecture.rs:163-256` — the source-level pins on the split

## Invariants

- Never shell out to `curl` from `snp`; never link eggfetch into `snip-sync`
  without re-running the size measurement.
- Redirect + total-timeout ownership stays delegated to eggfetch natives — no
  manual redirect loop, no outer Tokio timeout.
- HTTPS-only initial URLs in production on both sides; HTTP allowed only via
  test features against loopback fixtures.
- Test-only env vars (`SNIP_UPDATE_*`) are dead code in prod builds.
- Candidate replacement is verify-then-swap: SHA-256 sidecar (exact
  one-line `<64-hex>  <name>` format) plus a `version`-identity probe before
  the installed executable is touched.
- A failed or oversized download must never leave a usable partial candidate
  on disk.

## Gotchas

- **`--locked` is a no-op flag.** Both updaters bind it as `_locked`
  (`src/update.rs:80`, `snip-sync/src/update.rs:54`). The Cargo fallback
  always passes `--locked` to `cargo install` anyway
  (`src/update.rs:641-650`, `snip-sync/src/update.rs:417-423`), so the effective
  behavior is "always locked".
- **`eggfetch-core` is pinned with `=`, not `^`.** `Cargo.toml:142` uses an
  exact version. The manifest currently says `=0.2.2`, and the architecture
  test asserts the same pin (see [Architecture Pins](#architecture-pins)).
- **The `curl` adapter has no explicit redirect-depth bound.** It relies on
  curl's own default (`--max-redirs` is not passed), while the client's
  eggfetch path is explicitly bounded to 10 hops
  (`src/update.rs:279`). The two transports are not behaviorally identical.
- **The client's `Timeout.total` is per logical request**, not per hop and not
  per body chunk. A redirect chain does not get a fresh budget.
- **The staging file is created only after the final status is classified**
  (`src/update.rs:476-482`), so a 404 or 5xx never truncates or creates it.
- **`snip-sync` stops the running server before replacing the binary on
  Windows** (`snip-sync/src/update.rs:97-104`); on Unix the restart is
  attempted after the swap (`snip-sync/src/update.rs:108-111`). The client
  has no such lifecycle handling — it never runs a server.
- **The server's dry-run mentions the lifecycle state** because the server's
  update has side effects on a running service, unlike the client's.

## Cross-References

- [server.md](server.md) — the `snip-sync` server, including its own updater
  context.
- [.skills/server-module.md](../.skills/server-module.md) — the server
  updater's `curl` rationale.
- [cli.md](cli.md) — the CLI surface and exit codes.
