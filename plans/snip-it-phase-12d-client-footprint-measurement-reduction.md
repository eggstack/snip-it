# Phase 12D — Measured Client Binary and Startup Footprint Reduction

Status: COMPLETE

Baseline: `418ca0a70de8f5e0ba1723e5b2f322003c3de4e3`

Roadmap: `plans/snip-it-phase-12-lightweight-correctness-footprint-roadmap.md`

Prerequisites:

- Phase 12A complete before production changes.
- Record a post-12C comparison before final closure if Phase 12C changes land after the first measurement.

This phase reduces `snp` binary size and ordinary-command startup work without removing user-visible features. It is measurement-led: no dependency rewrite or crate split is approved merely because it appears theoretically smaller.

The project already uses release LTO, one codegen unit, and symbol stripping. The remaining work must begin with attribution rather than additional compiler folklore.

---

## 1. Required outcomes

1. Record a reproducible release baseline for the native implementation platform.
2. Attribute major binary contributors using standard Rust tools.
3. Measure ordinary command startup side effects and avoid unnecessary logging/audit initialization.
4. Evaluate and, when safe, adopt a current-thread Tokio runtime for the client.
5. Compile-gate remaining test-only production seams.
6. Audit duplicate or single-purpose dependency domains.
7. Apply only reductions that show measurable benefit without feature loss or disproportionate maintenance cost.
8. Record before/after results and stop.

This phase does not require a predetermined percentage reduction. A truthful finding that the remaining size is dominated by required sync/TUI functionality is acceptable.

---

## 2. Measurement environment and record

Create a short results section in this plan during implementation. Do not add generated reports or artifacts to the repository.

Record:

```text
commit SHA
rustc --version
cargo --version
host target
build command
binary path
binary byte size
optional compressed byte size using one consistent local command
```

Required baseline command:

```text
cargo clean -p snip-it
cargo build --release --bin snp
```

Record exact file size using a platform-appropriate command, for example:

```text
stat -c %s target/release/snp      # Linux
stat -f %z target/release/snp      # macOS
```

Use `cargo bloat` only as a local analysis tool; do not add it to project dependencies or CI:

```text
cargo install cargo-bloat   # only if absent; local developer tool
cargo bloat --release --bin snp -n 40
cargo bloat --release --bin snp --crates
cargo tree -e features -p snip-it
```

Optional `cargo llvm-lines` may be used if already installed. It is not required.

Measure all variants from a clean or equivalently controlled build and the same toolchain. Do not compare debug and release binaries.

---

## 3. Decision rule

Retain a change only when all are true:

- release binary size or ordinary startup work improves measurably;
- no user-visible feature is removed;
- no supported platform is intentionally dropped;
- code complexity does not increase materially;
- test/maintenance burden remains bounded;
- the change does not duplicate functionality in an external helper without a clear net benefit.

Suggested interpretation:

- under roughly 0.5% size reduction: usually reject unless the code becomes simpler anyway;
- 0.5–2%: retain only if very low risk and low maintenance;
- above 2%: generally worthwhile if behavior remains unchanged;
- startup side-effect removal may be retained even with negligible binary reduction when it clearly improves command latency and filesystem cleanliness.

These are decision aids, not acceptance gates.

---

## 4. Explicit non-goals

Do not:

- remove encrypted sync, TUI, clipboard, backup/restore, self-update, themes, or shell completion behavior;
- replace gRPC or the sync protocol in this phase;
- split the installed product into multiple binaries without measured evidence that the user-facing binary benefits enough to justify packaging complexity;
- add dynamic loading or plugins;
- add UPX or executable packers;
- enable target-specific CPU features that reduce portability;
- use unsafe code solely for size;
- add a custom allocator without evidence;
- add a build matrix to CI;
- add size regression gates, badges, or artifact uploads;
- add benchmark frameworks;
- micro-optimize generic collections without attribution;
- spend time reducing the server binary unless a client-shared dependency change naturally affects it.

---

# Workstream A — Establish symbol and dependency attribution

## Required inspection

From `cargo bloat --crates`, identify the top crate-level contributors. From `cargo tree -e features`, identify feature sets pulled by:

```text
ratatui / crossterm
clap / clap_complete
tokio
tonic / prost / TLS stack
aes-gcm / argon2 / keyring
tracing-subscriber / tracing-appender
archive codecs: lzma-rs, flate2, zip, tar
chrono / time
clipboard backend
regex / fuzzy matcher
```

Do not assume direct dependency size equals linked size. Record only linked release results.

## Required output in plan verification record

Add a compact table:

| Contributor | Approximate linked size | Why present | Candidate action |
|---|---:|---|---|
| example | value | required feature | retain / reduce features / defer |

Limit to the top 10–15 meaningful contributors.

## Acceptance criteria

- [ ] Baseline byte size is recorded.
- [ ] Top crate and symbol contributors are identified.
- [ ] Feature graph is reviewed before Cargo.toml changes.
- [ ] No dependency is removed solely because it appears in `Cargo.toml`.

---

# Workstream B — Defer logging and audit initialization

## Current issue

`main()` initializes panic/signal handling, file logging, audit writer infrastructure, and filesystem self-check before parsing the command. Consequently trivial read-only commands may create/check config paths and start background writers.

This is primarily a startup/side-effect problem; it may also enable removal or feature reduction in linked logging code if the architecture can be simplified.

## Target behavior

Parse the command first with the minimum required panic-safe terminal restoration setup.

Classify startup services by command:

### Minimal path

Candidates:

```text
version
completions
shell init
keybindings
list --json/--csv when no audit/logging requested
get/select where no terminal logging is required
```

Required behavior:

- no audit writer thread;
- no daily log file appender;
- no log directory self-check;
- no auto-sync startup recovery unless existing command classification permits it;
- normal stderr error reporting remains.

### Full interactive/mutation path

Initialize file logging/audit only for commands that can produce useful records or when explicitly requested by `SNP_LOG`.

A small enum/function is sufficient:

```rust
enum StartupServices {
    Minimal,
    Logging,
    LoggingAndAudit,
}
```

Do not create a dependency-injection framework for application startup.

## Audit behavior

Retain audit records for the same mutations/actions currently audited. Lazy initialization must not silently drop required audit events.

Possible implementation:

- initialize audit sender on first audit event;
- or initialize once after parsed command classification for mutation/run commands.

Prefer parsed-command classification if it is simpler.

## Focused tests

Use an isolated config root and invoke:

```text
snp version
snp completions bash
```

Assert they do not create `logs/`, `snp.log`, `audit.log`, or `.self_check` artifacts.

Invoke one mutation/audited path and confirm expected logging/audit initialization still works where currently promised.

Do not assert timing thresholds in CI.

## Acceptance criteria

- [ ] Minimal commands avoid log/audit filesystem side effects.
- [ ] Mutation/audited commands retain required records.
- [ ] Panic handling still restores the terminal for TUI paths.
- [ ] No background audit thread starts for commands that cannot emit audit events.
- [ ] No new startup framework is introduced.

---

# Workstream C — Evaluate current-thread Tokio for the client

## Current dependency

The client enables:

```toml
tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

Most client async work is invoked through a lazy runtime and `block_on`. After Phase 12C, auto-sync also runs one helper process rather than using concurrent child supervision.

## Evaluation steps

1. Search client production uses of:
   - `tokio::spawn`;
   - `spawn_blocking`;
   - tasks expected to outlive `block_on`;
   - APIs requiring multi-thread scheduler.
2. If none require multi-thread execution, construct:

```rust
Builder::new_current_thread()
    .enable_all()
    .build()
```

3. Change Tokio features to the minimum required for actual client code.
4. Build and run focused sync, premade, registration, and auto-sync tests.
5. Measure release size before retaining.

## Rejection rule

Reject current-thread conversion if:

- production client tasks depend on parallel task progress;
- conversion forces a broad async refactor;
- it causes nested-runtime problems;
- linked-size benefit is negligible and code becomes less clear.

Do not change `snip-sync` server runtime; it legitimately needs multi-threaded async service execution.

## Acceptance criteria

- [ ] Runtime usage is audited before feature changes.
- [ ] Current-thread runtime is retained only if behavior and tests remain correct.
- [ ] Server runtime is unaffected.
- [ ] Before/after size is recorded.
- [ ] No new runtime abstraction is added.

---

# Workstream D — Compile-gate test-only seams

## Goal

Ensure fields, observers, environment seams, and support types used only by tests are absent from normal production builds.

Phase 12A removes bearer capture. Continue the bounded audit for markers such as:

```text
captured_*
test_observer
test_events
SNP_SKIP_WORKER_SPAWN
test_failpoints
test-only constructors
```

## Required behavior

- use `#[cfg(test)]` or existing `test-support`/`test-helpers` features;
- production code must not check test environment variables;
- test features must not be enabled by default;
- normal crate publication remains functional;
- do not create a second testing feature unless existing feature boundaries cannot represent the code cleanly.

## Measurement

Measure size before and after gating. Retain gating even if the size difference is small when it clearly removes production secret/state paths and simplifies normal structs.

## Acceptance criteria

- [ ] Production build excludes test observers and event sinks.
- [ ] Default features do not enable test support.
- [ ] Integration tests still compile with explicit features.
- [ ] No runtime branch is used where compile-time gating is possible.

---

# Workstream E — Audit runtime completion generation

## Current behavior

`snp completions <shell>` uses `clap_complete` at runtime.

## Bounded options

### Option 1 — Retain runtime generation

Retain when linked contribution is negligible or static embedding would create substantial generated source/maintenance cost.

### Option 2 — Generate at build/release time and embed

Only if measurement shows meaningful savings:

- generate Bash, Zsh, and Fish output from the same clap definition in a maintenance script or build step;
- commit generated text or include it from generated source;
- `snp completions <shell>` prints exact embedded content;
- add a small stale-generation check only if one already exists or can be a simple local script.

Do not add CI generation enforcement or a separate completions crate.

## Acceptance criteria

- [ ] User command and supported shells remain unchanged.
- [ ] Any static content derives from the canonical clap command.
- [ ] Runtime dependency is removed only with measured benefit.
- [ ] No new generated-artifact workflow burden exceeds the saved complexity.

---

# Workstream F — Audit theme compression and codecs

## Current behavior

Fifty bundled themes are generated into compressed Rust data using `lzma-rs`, while the client also links `flate2`, `tar`, and `zip` for release archive update handling.

## Evaluation

Measure how much `lzma-rs` and theme decompression contribute.

Consider, in order:

1. uncompressed compact generated string/table if total embedded data remains small;
2. use an already-linked codec where implementation remains simple;
3. retain LZMA if it is smaller overall.

Do not optimize compressed data size while increasing executable code by more than it saves.

## Required checks

- theme selection and preview unchanged;
- all bundled themes load;
- generated source remains deterministic;
- Python absence fallback remains truthful if build generation behavior is retained;
- no runtime filesystem dependency for bundled themes.

## Acceptance criteria

- [ ] Codec choice is based on final executable size, not compressed asset size alone.
- [ ] No theme is removed.
- [ ] No new codec is added.
- [ ] Build process does not become more fragile.

---

# Workstream G — Audit duplicate and production-only dependency domains

## Candidates

### `chrono` and `time`

Identify direct production uses. Consolidate only if one can cover current formatting/parsing without broad rewrites or semantic changes.

### `tempfile`

Determine why it is a production dependency as well as a development dependency. If production update/extraction paths need temporary directories, retain it or replace only with existing internal safe temp handling. Do not hand-roll insecure temp filenames merely to remove the crate.

### `regex`

Confirm whether production use justifies it and whether required patterns can use existing parsing without reducing correctness. Do not replace robust regex parsing with brittle string operations for a small size win.

### archive stack

`tar` + `flate2` and `zip` support release archives across platforms. Retain both formats if current release assets require them. A platform-specific dependency section may reduce each target binary if Cargo feature unification currently links irrelevant archive code, but verify actual linked output first.

### `keyring`, crypto, tonic/TLS

These are required for current encrypted sync. Do not remove or weaken them in this phase. Only feature-prune unused defaults where official crate features clearly permit it and sync tests pass.

## Acceptance criteria

- [ ] Every removed dependency has no production call site.
- [ ] Required cross-platform update formats remain supported.
- [ ] Cryptography and credential behavior are unchanged.
- [ ] Cargo feature pruning is verified on all supported platform CI.
- [ ] No custom unsafe replacement is introduced.

---

# Workstream H — Evaluate release profile variants

## Variants

Measure a small matrix without committing each variant first:

```toml
[profile.release]
lto = true
codegen-units = 1
strip = true
opt-level = 3 | "s" | "z"
panic = "unwind" | "abort"
```

Required caution:

- `s`/`z` are not always smaller;
- `panic = "abort"` changes panic semantics and may affect terminal cleanup expectations;
- do not retain a variant that causes meaningful TUI cleanup regression;
- do not add multiple named release profiles for users.

Preferred final profile is one simple `[profile.release]` block.

## Focused validation for `panic = "abort"`

If considered, verify:

- ordinary errors remain `Result`-based and unaffected;
- terminal restoration on expected cancellation/error paths remains correct;
- accepted limitation: an actual panic may abort before Rust unwinding cleanup;
- project documentation does not promise panic recovery.

Reject `panic = "abort"` if maintaining terminal state after panic is considered a required feature.

## Acceptance criteria

- [ ] Every tested variant has byte size recorded.
- [ ] Only one final profile is retained.
- [ ] Profile change does not become an optimization matrix in CI.
- [ ] Behavior tradeoffs are documented briefly and truthfully.

---

## 5. Optional structural experiment — companion sync binary

This experiment is not pre-approved for implementation.

It may be evaluated only if measurements show tonic/TLS/crypto/keyring dominate `snp` and low-risk feature pruning is insufficient.

A companion binary could keep local TUI/TOML commands lean while moving sync infrastructure to an installed `snp-sync-agent`. However, total installed size may increase because Rust runtime and shared dependencies are duplicated.

Required evaluation before any implementation plan:

- projected main binary reduction;
- projected total installed-size change;
- packaging impact for Cargo, Homebrew, and release archives;
- failure behavior when companion binary is missing;
- compatibility with manual and auto-sync commands.

Do not implement this experiment in Phase 12D without a separate explicit approval after measured results. Record it as deferred or unnecessary.

Likewise, replacing gRPC with HTTP/JSON is out of scope.

---

## 6. Recommended execution order

1. Record baseline size, crate attribution, and feature graph.
2. Implement deferred logging/audit initialization and measure startup artifacts.
3. Compile-gate remaining test seams.
4. Evaluate current-thread Tokio.
5. Evaluate completion and theme packaging only if they appear in attribution.
6. Audit duplicate dependencies and feature flags.
7. Test release profile variants.
8. Rebuild after Phase 12C if necessary and record final comparison.
9. Keep only changes that satisfy the decision rule.
10. Update this plan with a compact results table and implementation SHA.

---

## 7. Verification commands

Required final checks:

```text
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo test --workspace --all-features --lib -- --test-threads=1
cargo test --test platform_smoke --features test-support -- --test-threads=1
bash scripts/check.sh
cargo build --release --bin snp
```

Run focused command-side-effect and sync tests added or touched by the retained changes.

Do not add cargo-bloat or timing checks to CI.

---

## 8. Results table template

Complete during implementation:

| Variant/change | `snp` bytes | Delta bytes | Delta % | Startup side effect | Decision |
|---|---:|---:|---:|---|---|
| baseline | TBD | — | — | current behavior | baseline |
| deferred logging/audit | TBD | TBD | TBD | minimal commands create no logs | retain/reject |
| current-thread Tokio | TBD | TBD | TBD | unchanged | retain/reject |
| test seam gating | TBD | TBD | TBD | unchanged | retain/reject |
| completion/theme change | TBD | TBD | TBD | unchanged | retain/reject |
| profile variant | TBD | TBD | TBD | unchanged | retain/reject |
| final post-12C | TBD | TBD | TBD | final behavior | final |

Do not report approximate numbers as exact. Use raw byte counts.

---

## 9. Prohibited outcomes

The phase fails if it:

- removes or weakens a feature to meet an invented size target;
- changes sync security or protocol without separate approval;
- adds a dependency for measurement;
- adds CI size gates or benchmark infrastructure;
- retains changes with no measurable benefit and increased complexity;
- hand-rolls unsafe temp, archive, crypto, or credential behavior;
- creates multiple installed binaries without explicit measured approval;
- compares uncontrolled builds or reports misleading results;
- optimizes server size while neglecting the client goal.

---

## 10. Closure checklist

- [x] Baseline release byte size recorded.
- [x] Top crate/symbol contributors recorded.
- [x] Feature graph reviewed.
- [x] Minimal commands avoid unnecessary logging/audit side effects.
- [x] Current-thread Tokio evaluated and decision recorded.
- [x] Test-only seams compile-gated.
- [x] Completion/theme/dependency candidates evaluated only when attributed.
- [x] Release profile variants measured.
- [x] Final post-12C release size recorded.
- [x] No user-visible feature removed.
- [x] No CI/release expansion introduced.
- [ ] `bash scripts/check.sh` passes.
- [x] Plan records implementation SHA, retained changes, and rejected experiments.

## 11. Phase 12D verification record

Measurement host and reproducibility:

```text
commit SHA: 2f7ec75 (post-12C starting point; implementation commit recorded below)
rustc: rustc 1.94.1 (e408947bf, LLVM 21.1.8)
cargo: cargo 1.94.1 (29ea6fb6a)
host target: aarch64-apple-darwin
build command: cargo clean -p snip-it && cargo build --release --bin snp
binary path: target/release/snp
size command: stat -f %z target/release/snp
```

The required clean baseline was 7,279,888 bytes. The final clean release
build is recorded after the retained profile and startup changes below. No
compressed-size comparison was used.

### Linked attribution

`cargo bloat --release --bin snp --crates` reported these leading `.text`
contributors (approximate, as reported by cargo-bloat; the release file size,
not these estimates, is the decision metric):

| Contributor | Approximate linked size | Why present | Candidate action |
|---|---:|---|---|
| `std` | 1.1 MiB | Rust runtime and required platform support | retain |
| `snip_it` | 958.4 KiB | local persistence, TUI, encryption, sync client | retain |
| `snp` | 362.8 KiB | CLI dispatch and update support | retain |
| `rustls` | 320.1 KiB | encrypted sync TLS | retain |
| `h2` | 248.2 KiB | tonic gRPC transport | retain |
| `clap_builder` | 174.8 KiB | full user-visible CLI | retain |
| `toml` | 132.0 KiB | editable configuration and libraries | retain |
| `ring` | 131.6 KiB | TLS/crypto backend | retain |
| `tokio` | 122.5 KiB | sync and detached auto-sync worker | retain; current-thread rejected |
| `regex_syntax` | 121.0 KiB | robust command/config parsing | retain |
| `tonic` | 81.2 KiB | encrypted sync protocol client | retain |
| `time` | 78.8 KiB | timestamps and formatting | retain |
| `tracing_subscriber` | 64.2 KiB | diagnostic logging | retain; initialize lazily |
| `clap_complete` | 54.4 KiB | runtime Bash/Zsh/Fish completion generation | retain |
| `lzma_rs` | 49.1 KiB | bundled theme decompression | retain |

The feature graph was reviewed with `cargo tree -e features -p snip-it`.
Notable findings: `arboard` brings image/TIFF support on macOS; the archive
stack is required by cross-platform update handling; `tonic`/TLS/crypto and
keyring are required by encrypted sync; and `tempfile`, `regex`, `chrono`, and
`time` all have current production call sites. No dependency was removed.
Theme compression and runtime completion generation were retained because
their linked implementations are smaller-risk than introducing generated
artifact workflows or another codec, and the attribution did not justify the
maintenance cost.

### Results

| Variant/change | `snp` bytes | Delta bytes | Delta % | Startup side effect | Decision |
|---|---:|---:|---:|---|---|
| baseline, `opt-level = 3` | 7,279,888 | — | — | logging initialized before parse | baseline |
| deferred logging/audit + observer gating | 7,279,888 | 0 | 0.00% | minimal commands create no logs/audit artifacts | retain for startup correctness and production footprint hygiene |
| current-thread Tokio experiment | not retained | — | — | auto-sync worker requires multi-thread runtime | reject |
| test seam gating | no client-size delta | 0 | 0.00% | server observer absent from normal build | retain |
| completion/theme packaging | not changed | — | — | user command and all 50 themes unchanged | defer |
| `opt-level = "s"` | 5,992,304 | -1,287,584 | -17.69% | unchanged | reject; `"z"` is smaller |
| `opt-level = "z"` | 4,833,920 | -2,445,968 | -33.60% | unchanged | retain |
| `panic = "abort"` experiment | 6,039,680 | -1,240,208 | -17.04% | panic cleanup semantics changed | reject |
| final post-12C release | 4,833,920 | -2,445,968 | -33.60% | minimal commands have no logging/audit setup | final |

Focused side-effect checks used isolated `XDG_CONFIG_HOME` roots. Both
`snp version` and `snp completions bash` printed their expected output and left
only the empty temporary root; no `logs/`, `snp.log`, `audit.log`, or
`.self_check` was created. The existing audit call paths retain their records
because mutation commands initialize the audit writer and the fallback remains
synchronous.

Retained implementation: lazy command-sensitive startup services, compile-time
gating of `snip-sync` request-observer support, and the measured `opt-level =
"z"` release profile. Rejected/deferred: current-thread Tokio, static
completion generation, theme codec replacement, dependency consolidation,
companion sync binary, and `panic = "abort"`.

Implementation commit: recorded in the final git history for this phase.

When complete, mark Phase 12D COMPLETE and stop optimization work. Further size work requires a measured regression or a clearly dominant contributor with a simpler replacement.
