# Eggstack Dependency Refresh M001 — Closure Status

Status: closed

Source implementation plan:

- `plans/implementation/eggstack-dependency-refresh/001-upgrade-eggfetch-and-eggserve.md`

Source subsystem roadmap:

- `plans/subsystems/eggstack-dependency-refresh-roadmap.md` M001

Repository baseline reviewed: `19200759b7f7b382c4d7d6b36952c64645baab96`
(matches the implementation plan's stated baseline; branch
`codex/eggstack-dependency-upgrades-20261008` planning commits
`5871ee0`–`70bea94` on top, reconfirmed via `git merge-base HEAD main`
before editing).

Implementation commits or pull requests:

- This closure commit on `codex/eggstack-dependency-upgrades-20261008`
  (pins, lockfile, guard/docs reconciliation, closure record, roadmap and
  registry updates; SHA recorded at push time).
- No pull request: topic branch to be squash-merged to `main` per release
  governance. No publication is part of this milestone.

Primary class: infrastructure (dependency refresh, not new capability).
Hard dependencies: updater-transport M005 closed, server-lifecycle-http
M004 closed. No new ADR (third-party versions only; ownership
architecture unchanged).

Toolchain: `rustc 1.94.1`, `cargo 1.94.1`, same host/toolchain/profile
for baseline vs after comparison.

## 1. Executive finding

M001 is closed. Exact pins `eggfetch-core =0.2.2`, `eggserve-server
=0.4.0`, `eggserve-primitives =0.2.2` resolve in `Cargo.lock` with no
legacy `eggserve-primitives 0.2.1` copy, the untouched downstream source
compiles against the new versions with zero Rust source changes, all
focused updater (21), orchestration (13), architecture-guard (6), HTTP
wire/lifetime (6 + 2 long-lived), and gate suites pass, and same-host
release artifacts are byte-identical to the fresh baseline (`snp`
6,842,056 bytes, `snip-sync` 3,898,456 bytes; 0.00% delta on both).
Historical measurements (6,776,320; 3,833,152 → 5,145,224 +34.23%) are
retained as history and were not treated as the current baseline.

## 2. Requirement-to-evidence matrix

| Requirement | Evidence | Result | Notes |
|---|---|---|---|
| Pins resolve exactly to 0.2.2 / 0.4.0 / 0.2.2, no legacy primitives copy | `Cargo.toml`, `snip-sync/Cargo.toml`, `Cargo.lock`, `cargo tree -i eggserve-primitives` (single 0.2.2 via server 0.4.0 + direct) | pass | Updated only via `cargo update -p <crate> --precise`; no hand-edited lockfile. |
| Full workspace compiles on declared toolchain; published-feature compatibility proven by build | `cargo check --workspace --all-targets --locked` exit 0; `cargo tree -p snip-it/snip-sync -e features` | pass | Zero Rust source adaptations required; research disposition confirmed. |
| Lean dependency profile preserved; no framework/split violations | Feature trees: eggfetch `standard-http1`+`redirects`+`tls-rustls`+`tls-native-roots`, no broad `http1` alias; eggserve subtree has no `tower-http`, no `axum` (axum present only via pre-existing `tonic` path), no server-side `eggfetch-core`; server updater still external `curl` | pass | `tests/architecture.rs` lean-profile, no-curl, and delegated-timeout guards green. |
| Updater bounded strict redirection/transport, integrity, fail-closed fallback | `cargo test -p snip-it --bin snp --features test-support update::tests`: 21 passed | pass | Covers redirect chain/depth, downgrade denial, metadata/binary caps, streaming, slow-drip total timeout incl. EOF, staging cleanup, 404-only fallback, checksum/candidate checks. |
| Real-socket health/metrics parity, body rejection, CORS/security/auth, shutdown correctness | `cargo test --test snip_sync_lifetime -- --test-threads=1`: 6 passed, 2 ignored; `--ignored`: 2 passed; `cargo test -p snip-sync --lib orchestration`: 13 passed | pass | Exact `Vary`/`Allow`/empty-404-405/challenge/header-boundary/pre-bound/startup-drain/typed-completion behavior preserved on server 0.4.0. |
| No gRPC sync regression from lockfile graph change | `cargo test --workspace --lib` (1171 + 147 passed) plus `sync_multibatch` 6 passed; `cargo test -p snip-sync --features test-helpers` (147 + 2 + 2 passed) | pass | No proto/wire change; smoke is existing client/server paths. |
| Release-size and dependency-feature deltas documented | Fresh baseline vs after builds `--locked`, same host/toolchain/profile | pass | Both binaries byte-identical (0.00%); no investigation threshold crossed. |
| Architecture test and live docs agree with new pins; old measurements keep historical meaning | `tests/architecture.rs` guard now asserts `=0.2.2`; `architecture/update.md`, `architecture/server.md`, `architecture/overview.md`, `plans/000-long-term-specification.md` §8 reconciled; historical values annotated | pass | Canonical §8 changed narrowly to version-neutral wording + live-manifest reference; §§1–7 untouched; archived closures untouched. |
| Gated verification and production-seam checks | `scripts/check.sh` components (see §4) and `scripts/ci/test-production-seams.sh` all PASS | pass | Two load-induced timing flakes recorded honestly in §4/§10; green on rerun. Full single-pass `check.sh` exceeded the 20-minute tool budget, so evidence is per-step with identical flags. |

## 3. Production implementation evidence

Landed production changes (no Rust source changes):

- `Cargo.toml`: `eggfetch-core` `=0.2.0` → `=0.2.2`; feature list
  unchanged (`default-features = false`; exactly `standard-http1`,
  `redirects`, `tls-rustls`, `tls-native-roots`).
- `snip-sync/Cargo.toml`: `eggserve-server` `=0.2.1` → `=0.4.0`,
  `eggserve-primitives` `=0.2.1` → `=0.2.2`; defaults off preserved.
- `Cargo.lock`: `eggfetch-core 0.2.0→0.2.2`, `eggserve-server
  0.2.1→0.4.0`, `eggserve-primitives 0.2.1→0.2.2` (sole copy).
- `src/update.rs`, `snip-sync/src/http.rs`, `orchestration.rs`,
  `main.rs`: unchanged (no API adaptation demonstrated necessary).
- Guard/docs reconciliation: `tests/architecture.rs` pin assertion and
  comment; `architecture/update.md`, `architecture/server.md`,
  `architecture/overview.md` current-version statements (historical
  measurements annotated, not rewritten); `plans/000-long-term-
  specification.md` §8 minimal version-neutral rewording with live-manifest
  reference and M001 rationale.

Planned but absent (per non-goals, correctly absent): Eggress, HTTP/2/3,
in-server TLS termination, Axum/Tower adoption, new crates/adapters,
blanket refresh, protocol/schema changes, codegen, server updater rewrite,
release publication/version bump.

## 4. Verification executed

### Commands run

```bash
rustc -V; cargo -V
cargo update -p eggfetch-core --precise 0.2.2
cargo update -p eggserve-server --precise 0.4.0
cargo update -p eggserve-primitives --precise 0.2.2
cargo tree -p snip-it -e features
cargo tree -p snip-sync -e features
cargo tree -i eggserve-primitives
cargo check --workspace --all-targets --locked
cargo test -p snip-it --bin snp --features test-support update::tests
cargo test -p snip-sync --lib orchestration
cargo test --test architecture
cargo test -p snip-sync --features test-helpers
cargo test --test snip_sync_lifetime -- --test-threads=1
cargo test --test snip_sync_lifetime -- --ignored --test-threads=1
cargo test --workspace --lib
cargo build --release -p snip-it --bin snp --locked
cargo build --release -p snip-sync --bin snip-sync --locked
bash scripts/tests/installers.sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
SNP_ALLOW_PLAINTEXT_API_KEY=true cargo test --test platform_smoke --features test-support
SNP_ALLOW_PLAINTEXT_API_KEY=true cargo test --test destination_permissions --features test-support
SNP_ALLOW_PLAINTEXT_API_KEY=true cargo test --test auto_sync_closure --features test-support
SNP_ALLOW_PLAINTEXT_API_KEY=true cargo test --test auto_sync_concurrency --features test-support -- --test-threads=1
SNP_ALLOW_PLAINTEXT_API_KEY=true cargo test --test sync_multibatch --features test-support -- --test-threads=1
cargo build -p snip-sync --bin snip-sync
SNP_ALLOW_PLAINTEXT_API_KEY=true bash scripts/ci/test-production-seams.sh
```

`scripts/check.sh` as a single command was attempted with
`SNP_ALLOW_PLAINTEXT_API_KEY=true` but exceeded the 20-minute execution
budget in this environment; every step it performs is covered above with
identical flags (see results). `scripts/release-check.sh verify` was not
run: per the implementation plan it is manual pre-release only, not a
prerequisite for this maintenance milestone.

### Results

| Command | Outcome |
|---|---|
| `cargo update -p … --precise` (3x) | First call resolved all three pins; other two no-ops. |
| `cargo tree` inspections | Single `eggserve-primitives 0.2.2`; lean eggfetch features; no `tower-http`/server-`eggfetch-core`; `axum` only via pre-existing `tonic` path. |
| `cargo check --workspace --all-targets --locked` | Pass. |
| `update::tests` | 21 passed, 0 failed. |
| `snip-sync --lib orchestration` | 13 passed, 0 failed. |
| `cargo test --test architecture` | 6 passed, 0 failed (updated `=0.2.2` guard green). |
| `cargo test -p snip-sync --features test-helpers` | 147 + 2 + 2 passed, 0 failed. |
| `snip_sync_lifetime` (serial) | 6 passed, 2 ignored; `--ignored` rerun 2 passed. |
| `cargo test --workspace --lib` | snip-it lib 1171 passed, 8 ignored; snip-sync lib 147 passed. See flake note (a). |
| Release builds `--locked` | `snp` 6,842,056 B; `snip-sync` 3,898,456 B — both byte-identical to fresh baseline (0.00%). |
| `installers.sh` | Exit 0, "installer contract tests passed" (failure-path lines are negative cases). |
| `cargo fmt --all -- --check` | Exit 0. |
| `cargo clippy --workspace --all-targets -- -D warnings` | Exit 0. |
| `platform_smoke` | 16 passed. |
| `destination_permissions` | 18 passed. |
| `auto_sync_closure` | 11 passed. |
| `auto_sync_concurrency` (serial) | Rerun 28 passed. See flake note (b). |
| `sync_multibatch` (serial) | 6 passed. |
| `snip_sync` dev build + `snip_sync_lifetime` rerun | Build ok; 6 passed, 2 ignored. |
| `test-production-seams.sh` | All 5 seam checks PASS, exit 0. |

Flake notes (recorded, not concealed):

- (a) First `check.sh`-context full lib run: 1170 passed, 1 failed in
  `auto_sync::schedule::tests::test_mutation_during_backoff_does_not_spawn`
  (`mutation 0 should be deferred, got SpawnNow`). Isolated rerun passed;
  full `cargo test -p snip-it --lib` rerun passed (1171 passed, 8
  ignored). No auto-sync or scheduling source was touched by this
  milestone; the test is timing-sensitive under parallel load.
- (b) First `auto_sync_concurrency` serial run: 27 passed, 1 failed over
  362 s (failing test name not captured); rerun passed 28/28 over ~90 s.
  Same assessment as (a): load-induced timing flake in an untouched
  subsystem, green on rerun.

Cross-platform: Linux proven locally. macOS/Windows compile checks were
not run in this environment; hosted CI on push is the verification path
(see §10, low severity).

## 5. Invariant review

| Plan invariant | Evidence it remains true |
|---|---|
| `snp update`: HTTPS-only initial URL, strict max-10 redirects, downgrade denial pre-I/O, 60 s total deadline incl. redirect hops and all body bytes, bounded metadata/binary streams, no decompression, no hidden retries, 404-only Cargo fallback, staging cleanup | 21 focused `update::tests` green on 0.2.2; `RedirectPolicy::strict` + native `Timeout.total` + initial-scheme guard untouched; architecture guards green. |
| `snip-sync`: concrete two-route H1 service, Tonic separate, pre-bound listeners, Eggserve-owned body rejection, no connection-lifetime cap, HEAD/OPTIONS, CORS `Vary: origin`, `Allow`, status/body/content-type, security headers, Basic-auth + `WWW-Authenticate` exact | 6 + 2 real-socket parity tests green on server 0.4.0/primitives 0.2.2; `http.rs`/`orchestration.rs`/`main.rs` untouched. |
| Shutdown: `ServerControl` vs single-owner `ServerCompletion`, cancellation-safe `select!`, early-failure propagation, dual-clean-exit only on success, bounded drain, no detached task | 13 orchestration tests green, incl. drain/panic/error classification. |
| Lightweight split: defaults off, eggfetch features unchanged, no server eggfetch/framework, server updater stays `curl` | Feature-tree inspection + no-curl guard + `snip-sync` tree free of eggfetch; byte-identical binaries. |
| No storage/protobuf/tag/CLI/protocol change | No migration code touched; `Cargo`-only diff outside docs/guards; sync smoke suites green. |

## 6. Failure and recovery review

No new protocol or persisted state. Applicable behaviors re-proven by
existing tests on the new pins: failed update HTTP/checksum leaves the
installed binary untouched with staging cleaned (updater tests); server
bind failure fails fast with no half-started transport (lifetime
pre-bound tests); shutdown control drains once with typed outcomes
(orchestration tests); updater performs no retries (guard + tests).
Concurrency races remain covered by existing focused suites; flakes (a)
and (b) are load-induced timing sensitivities in untouched auto-sync
tests, not new failure modes.

## 7. Migration and compatibility review

Cargo pin/lockfile-only change plus guard/docs reconciliation. Zero
migrations: no SQLite schema, protobuf, sync identity/merge, auth-key
storage, config, CLI, or release-tag change. Host targets and Rust 1.94
unchanged; no proto regeneration; no published version bump. Rollback is
the three pins plus lockfile. Historical version strings in accepted
closure records are unchanged; live normative/architecture guides no
longer assert obsolete pins.

## 8. Security review

TLS/redirect enforcement, credential-header behavior, metrics Basic
authentication (constant-time), request-body rejection, allowed origins,
and the three security headers are preserved and proven by the passing
wire/parity suites; no security assertion was weakened (failing tests
were rerun, never edited). No secret-handling, keychain, Argon2, or auth
logic was touched. Test-only env seams (`SNIP_UPDATE_*`,
`SNP_ALLOW_PLAINTEXT_API_KEY`, `SNP_TEST_*`) verified inert in
production builds via `test-production-seams.sh` (all PASS).

## 9. Documentation and operations

- `tests/architecture.rs`: pin guard and comment updated to `=0.2.2`
  with history note.
- `architecture/update.md`: current-version statements updated;
  historical trial/measurement annotated as retained history.
- `architecture/server.md`: dependency statements updated to
  `=0.4.0`/`=0.2.2`; historical size-gate annotated.
- `architecture/overview.md`: client pin updated; historical trial
  annotated.
- `plans/000-long-term-specification.md` §8: minimal version-neutral
  rewording with live-manifest reference and M001 rationale; normative
  four-feature/security policy preserved; §§1–7 and archived closures
  untouched.
- Static guards: `snp_updater_pins_lean_eggfetch_profile`,
  `snp_updater_does_not_shell_out_to_curl`,
  `snp_updater_delegates_redirects_and_total_timeout` all green.
- Operations: Rust 1.94; Linux verified; macOS/Windows via hosted CI.

## 10. Unresolved findings

| Severity | Finding | Impact | Required action |
|---|---|---|---|
| low | macOS/Windows CI compile not run locally; hosted CI is the verification path | Platform proof gap outside this environment | Confirm hosted CI green after push; open a corrective only on failure. |
| low | Two load-induced timing flakes in untouched auto-sync suites ((a) schedule backoff unit test, (b) one `auto_sync_concurrency` case), both green on rerun | No product impact; rerun cost only | None for this milestone; consider separate test-hardening work if flakes recur on `main`. |

No critical/high/medium findings. No corrective pass required. No
blocked milestone.

## 11. Roadmap disposition

M001 closed. The Eggstack dependency refresh workstream has no further
milestones; its roadmap moves to closed. No publication follows from
this milestone; release qualification remains a separate future decision.

## 12. Registry updates

- `plans/registry.md`: Eggstack roadmap row `active` → `closed` (M001
  closed); dependency-ready row `ready` → `closed`; "Next handoff" no
  longer names this milestone (all workstreams closed, no blocked work);
  closure-work table gains the Eggstack M001 row pointing at this record.
- `plans/subsystems/eggstack-dependency-refresh-roadmap.md`: status
  `active (M001 ready)` → `closed (M001 closed)`; milestone table
  updated; completion definition satisfied.
- `plans/README.md`: Eggstack workstream row `ready` → `closed`.
- Blocked-work audit (registry step 8): no subsystem lists this
  workstream as a hard/interface dependency (verified by workspace-wide
  search for `eggstack-dependency` references: only this workstream's
  own roadmap/plan/registry/README entries). **No future plan is
  unblocked by this closure; none was blocked on it.** "Blocked work"
  remains empty. No dependent status required changes.
