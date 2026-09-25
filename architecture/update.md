# Self-Update Transport

[← Back to Overview](overview.md)

## Overview

`snp update` and `snip-sync update` share the same binary-first strategy —
crates.io selects the stable version, the exact component GitHub tag
supplies the binary + checksum, and the installed executable changes only
after the candidate passes integrity and identity checks — but they use
deliberately different transports. The client fetches in-process via
`eggfetch-core =0.2.0`; the server shells out to external `curl`. That split
is a measured footprint tradeoff, pinned by architecture tests.

**Sources**: `src/update.rs`, `snip-sync/src/update.rs`,
`src/main.rs:452-453` (`RUNTIME.block_on(update::run)`), pins in
`tests/architecture.rs:172-256`. (Line refs drift — verify with `grep`.)

## `snp update`: In-Process eggfetch-core

`run(dry_run, locked)` (`src/update.rs:80`): detect install method
(Cargo / Homebrew / direct, `src/update.rs:181`), compare
`CARGO_PKG_VERSION` against crates.io `max_version` (prereleases rejected,
`src/update.rs:238`), then for prebuilt hosts download the exact-tag asset
(`{tag_prefix}v{version}`, `src/update.rs:145-168`), verify SHA-256 sidecar
+ `version`-identity probe, and atomically replace the executable.

- **Pinned dependency** (`Cargo.toml:132`): `eggfetch-core = "=0.2.0"`,
  `default-features = false`, features exactly `standard-http1`,
  `redirects`, `tls-rustls`, `tls-native-roots` — the lean HTTP/1.1 route,
  no broad `http1` alias, no automatic decompression (release bytes must
  stay byte-for-byte).
- **Strict redirects, delegated**: `update_redirect_policy()`
  (`src/update.rs:323`) returns
  `RedirectPolicy::strict(MAX_REDIRECTS)` (10-hop bound,
  `src/update.rs:279`) with HTTPS→HTTP downgrade denial. One dispatch
  (`fetch_response`, `src/update.rs:358`) validates the initial scheme, then
  sends a single GET — redirect traversal is eggfetch's job, not a manual
  loop.
- **Delegated total timeout**: the per-request native `Timeout.total`
  (60 s, `FETCH_OVERALL_TIMEOUT`, `src/update.rs:287` — parity with the old
  `curl --max-time 60`) spans redirect traversal through final-body EOF and
  does not reset per hop or per chunk. Client-level connect (10 s) / read
  (60 s) defaults are retained (`src/update.rs:337-349`).
- **Initial-HTTPS guard**: `check_url_scheme()` (`src/update.rs:305`)
  rejects non-HTTPS initial URLs before any I/O; only `test-support` builds
  widen this to loopback `http://` via `build_allow_http()`
  (`src/update.rs:294`). The production seam is test-only — see below.
- **No retries** (`update_http_client`, `src/update.rs:333`): the updater
  must not hide transport/release defects. Cargo/Homebrew/candidate
  verification/Windows replacement remain the only subprocess uses — no
  `curl` anywhere in `src/update.rs`.
- Streaming downloads write chunk-by-chunk without buffering the asset
  (`stream_binary_to_file`, `src/update.rs:502`); failures remove the
  partial staging file (`src/update.rs:495`); 404 maps to the Cargo
  source-fallback, other non-2xx are hard errors.

## `snip-sync update`: External curl

`snip-sync/src/update.rs` keeps the `curl` adapter (`fetch_bytes`,
`snip-sync/src/update.rs:186`; `fetch_file`,
`snip-sync/src/update.rs:232`) with `--location`, `--proto =https`
(`curl_protocol()`, `snip-sync/src/update.rs:175`), `--tlsv1.2`,
connect 10 s / max 60 s, byte caps, and the same initial-HTTPS guard
(`validate_https_url`, `snip-sync/src/update.rs:162`; `test-helpers`
widens it to loopback HTTP, mirroring the client seam).

**Why**: Plan 017's controlled same-toolchain lean `eggfetch-core 0.2.0`
trial grew the deliberately small server binary from **3,833,152 → 5,145,224
bytes (+34.23%)**, past the plan's 10% material-growth gate. The module
header (`snip-sync/src/update.rs:7-13`) records this; do not consolidate the
adapter without re-running the measurement and preserving equivalent
behavior (checksum contract, identity probe, lifecycle-aware restart).

## Architecture Pins

`tests/architecture.rs` locks the split at source level:

| Test | Pin |
|------|-----|
| `snp_updater_does_not_shell_out_to_curl` (`:172`) | `src/update.rs` contains no `"curl"` / `curl_protocol` (server file explicitly excluded) |
| `snp_updater_pins_lean_eggfetch_profile` (`:201`) | `Cargo.toml` pins `=0.2.0` with `standard-http1` + `redirects` |
| `snp_updater_delegates_redirects_and_total_timeout` (`:221`) | `src/update.rs` uses `RedirectPolicy::strict`, contains no `tokio::time::timeout`, no `safe_get` / `is_redirect_status` / `redirect_location` / `redirect_target` / `follow_redirects(false)` |

## Transport Tests

Updater transport tests live in `src/update.rs` under `test-support`
(`src/update.rs:1029`) with a **std-only loopback fixture**
(`TcpListener` on `127.0.0.1:0`, `src/update.rs:1130`) — no Tokio test
runtime, no external network. Production-policy cases pass
`allow_http = false` explicitly so the HTTPS-only rule is proven in-tree;
the downgrade-rejection case is qualified by eggfetch 0.2.0 behavior and
asserts via `RedirectDowngradePolicy::Deny`
(`src/update.rs:1650-1683`). Endpoint overrides (`SNIP_UPDATE_CRATES_API_URL`,
`SNIP_UPDATE_RELEASE_BASE_URL`) exist only under `test-support` /
`test-helpers` (`src/update.rs:259`, `snip-sync/src/update.rs:150`) and are
inactive in production builds.

## Install Methods & Replacement

`detect_install_method()` (`src/update.rs:181`) classifies the running
binary as Homebrew (under `brew --prefix <formula>`), Cargo (under a
`bin/` with `.crates.toml` / `.crates2.json` nearby), or direct. Homebrew
installs delegate to `brew upgrade`; direct/Cargo paths download the
prebuilt asset for `host_target()` (`src/update.rs:522`: six prebuilt /
source-only mappings plus unsupported → Cargo fallback) with `--dry-run`
printing the exact tag/asset that would be fetched (`src/update.rs:145`).
Replacement stages the candidate beside the destination, fsyncs, preserves
permissions, and renames atomically; Windows goes through the hidden
`__self-replace` helper (`src/main.rs:458`) via a staged copy of the
running executable. `ensure_destination_writable()` probes with a
temp file before any download so permission failures fail fast.

## Invariants

- Never shell out to `curl` from `snp`; never link eggfetch into
  `snip-sync` without re-running the size measurement.
- Redirect + total-timeout ownership stays delegated to eggfetch natives —
  no manual redirect loop, no outer Tokio timeout.
- HTTPS-only initial URLs in production on both sides; HTTP allowed only
  via test features against loopback fixtures.
- Test-only env vars (`SNIP_UPDATE_*`) are dead code in prod builds.
- Candidate replacement is verify-then-swap: SHA-256 sidecar (exact
  one-line `<64-hex>  <name>` format) plus a `version`-identity probe
  before the installed executable is touched.
