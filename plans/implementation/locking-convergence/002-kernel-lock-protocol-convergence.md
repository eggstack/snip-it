# Locking Protocol and Ordering Convergence Milestone 002 — Kernel-lock protocol convergence

Status: ready

Repository baseline: `dca5d3136db8a216ba4fcf8bfddac9140d1029ff`

Source roadmap:

- `plans/subsystems/locking-convergence-roadmap.md#m002--kernel-lock-protocol-convergence`

Long-term requirements:

- `plans/000-long-term-specification.md#5-durability-invariants`
- `plans/000-long-term-specification.md#10-verification-and-evidence-rules`
- `plans/001-terminology-and-domain-model.md#4-library-and-persistence-terms`
- `plans/001-terminology-and-domain-model.md#5-sync-and-execution-terms`

Applicable ADRs: none.

Primary class: infrastructure

## 1. Objective

Make the kernel-backed file-lock protocol exist exactly once per platform in
the workspace, and make every caller that takes a kernel lock — the client's
auto-sync execution lock, the client's auto-sync pending-marker lock, the
`snip-sync` server singleton lock, and the `snip-sync` `croncheck.lock` —
acquire it through that one implementation and share one definition of which
errors mean contention.

This milestone does **not** touch the owned-record lock family. The
owned-record protocol (`transaction.lock`, `local-data.lock`) was unified by
M001 in `src/record_lock.rs` and is out of scope here; the two families must
not be merged.

## 2. Why this milestone is ready

- No hard dependencies. The roadmap records M002's dependency as "hard none;
  soft M001" (`plans/subsystems/locking-convergence-roadmap.md:258-259`).
  M001 is closed (`plans/closure/locking-convergence/001-status.md`), so the
  soft dependency is satisfied and its shared duplicate-removal convention is
  available to follow.
- The divergent copies are already located and their divergence is already
  observed in production code (§3), and the existing tests already pin the
  behavior that must be preserved: `src/process_file_lock.rs:530-537` pins
  the canonical busy classification, `snip-sync/src/server_lock.rs:667-678`
  pins the server mirror's retry classification, and
  `tests/process_lock_concurrency.rs`, `tests/auto_sync_concurrency.rs`, and
  `snip-sync/tests/server_findings.rs` already exercise the acquisition paths
  this milestone rewrites.
- No decision here requires an ADR. The milestone changes *where* the
  primitive lives and *how contention is classified*; it does not change
  which resources are locked, how long a lock is held, who may reclaim a
  stale lock, or the auto-sync/sync authority relationship — all of which the
  roadmap records as ADR triggers
  (`plans/subsystems/locking-convergence-roadmap.md:401-403`).

## 3. Current implementation evidence

### 3.1 The canonical implementation and its two copies

| Copy | File | Unix acquire | Windows `LockFileEx` | `Drop` / release |
|------|------|--------------|---------------------|------------------|
| Canonical | `src/process_file_lock.rs` | `flock` `:302` | `:325-333` | `:190-200`, `:203-210`, `:213-227` |
| Server singleton | `snip-sync/src/server_lock.rs` | `:205-221` | `:262-272` | `:121-129`, `:132-153` |
| `croncheck.lock` | `snip-sync/src/process.rs` | `flock` `:228` | `:265-273` | `:188-211` |

The canonical module also owns the busy classification (`:406-408`), the
identity publication (`:353-360`, `:410-418`), the `UnsupportedPlatform`
failure (`:349`), and the start-token helpers (`:431`, `:500-504`). The
Windows locked range (one byte at `u32::MAX`) is defined three times:
`src/process_file_lock.rs:230`, `:236`; `snip-sync/src/server_lock.rs:158`,
`:164`; `snip-sync/src/process.rs:306`, `:309`.

The Windows `LockFileEx` block is therefore copy-pasted **three** times. All
three agree today: same flags, same byte range, raw OS error `32`
(`ERROR_SHARING_VIOLATION`) or `33` (`ERROR_LOCK_VIOLATION`) → busy,
everything else → hard failure (`src/process_file_lock.rs:336-343`,
`snip-sync/src/server_lock.rs:271-277`,
`snip-sync/src/process.rs:274-280`). The Windows side is duplicated but has
**not** diverged.

Two further differences are verified: the canonical implementation opens the
file with `read(true).write(true).create(true).truncate(false)`
(`src/process_file_lock.rs:291-296`) while the `croncheck` copy omits the
read handle (`snip-sync/src/process.rs:222-226`); and only the canonical
implementation and the server mirror publish an identity record
(`src/process_file_lock.rs:353-360`, `snip-sync/src/server_lock.rs:34-47`) —
the `croncheck` copy publishes nothing
(`snip-sync/src/process.rs:230-234`).

The `croncheck` copy's only production caller is
`snip-sync/src/main.rs:647-648`. `architecture/process_file_lock.md` lists
the three client-side kernel locks (`:325-327`) and the server singleton
(`:349-369`) but has **no entry** for `croncheck.lock` — the roadmap says
the same (`plans/subsystems/locking-convergence-roadmap.md:124-126`).

### 3.2 The Unix copies have already diverged — the `EACCES` claim is verified

The roadmap and the registry both claim the copies "already disagree on
whether `EACCES` means 'busy' or 'fatal'"
(`plans/subsystems/locking-convergence-roadmap.md:127-133`,
`plans/registry.md:112-113`). **That claim is independently verified and is
true.** The three Unix classification sites are:

| Copy | Busy condition | `EACCES` | `EINTR` |
|------|----------------|----------|---------|
| Canonical — `src/process_file_lock.rs:406-408` | `EWOULDBLOCK \|\| EAGAIN \|\| EACCES` | **Busy** | fatal (`Io`) |
| Server mirror — `snip-sync/src/server_lock.rs:216-219` | `EWOULDBLOCK` only | **fatal** | retried (`:213-214`) |
| `croncheck` copy — `snip-sync/src/process.rs:236-243` | `ErrorKind::WouldBlock` only | **fatal** | fatal |

Evidence for each row:

- Canonical: `fn is_lock_busy_error(code: Option<i32>) -> bool` at
  `src/process_file_lock.rs:406-408` matches on `libc::EWOULDBLOCK ||
  libc::EAGAIN || libc::EACCES`. Its unit test at
  `src/process_file_lock.rs:530-537` asserts `EAGAIN`, `EWOULDBLOCK`, and
  `EACCES` are busy and that `EINTR` is **not** busy.
- Server mirror: `EWOULDBLOCK` alone is contention
  (`snip-sync/src/server_lock.rs:216-218`); every other errno reaches
  `return Err(error)` at `snip-sync/src/server_lock.rs:219`, so `EACCES`
  becomes `ServerLockError::Io`. Its own test
  `retryable_lock_error_is_only_eintr`
  (`snip-sync/src/server_lock.rs:667-678`) pins `EINTR` as the *only*
  retryable errno.
- `croncheck` copy: it tests
  `std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock`
  (`snip-sync/src/process.rs:236`). On Unix `EACCES` maps to
  `ErrorKind::PermissionDenied`, not `WouldBlock`, so it reaches the
  `Err(...)` arm at `snip-sync/src/process.rs:239-243`.

The divergence is real in **two** independent dimensions, not one: `EACCES`
(busy in the client, fatal in both `snip-sync` copies) and `EINTR` (fatal in
the client, retried in the server mirror, fatal in the `croncheck` copy).

### 3.3 The client's self-duplication

`src/auto_sync/execution_lock.rs` duplicates itself:

- `inspect` (`:166-168`) and `worker_inspect` (`:283-285`) have identical
  bodies: both call `process_file_lock::read_owner(path)`.
- `is_stale` (`:171-173`) and `worker_is_stale` (`:287-289`) have identical
  bodies: both compute `!process_alive(contents.pid)`.
- `process_alive` is defined **twice** under mutually exclusive `cfg`
  attributes — `#[cfg(unix)]` at `:179-182` and `#[cfg(not(unix))]` at
  `:188-191` — with identical bodies. The split is redundant: the body has no
  platform-specific code.
- The `UnsupportedPlatform` → `ErrorKind::Unsupported` conversion is
  re-implemented in three places: `src/auto_sync/execution_lock.rs:95-98`,
  `:237-240`, and `src/auto_sync/pending_lock.rs:64-67`.

### 3.4 Where the shared implementation can live

`snip-sync` does **not** depend on `snip-it`; the direction is the other way
(`snip-it` depends on `snip-sync` only as a dev-dependency, `Cargo.toml:166`),
and `snip-sync`'s dependency list (`snip-sync/Cargo.toml:35-63`) contains no
path dependency on the client package. The single implementation therefore
cannot live in `snip-it` and be reached by `snip-sync`.

| Option | Verdict |
|--------|---------|
| New published workspace crate (e.g. `snip-lock`) that both crates depend on | **Recommended.** Satisfies "exactly one implementation of each platform's lock primitive exists in the workspace" (`plans/subsystems/locking-convergence-roadmap.md:273-274`). Precedent: `snip-proto` is already a shared published crate (`architecture/overview.md:120-127`). |
| `snip-sync` depends on `snip-it` | Rejected. It would pull the whole client dependency tree (ratatui, crossterm, keyring, argon2, aes-gcm, eggfetch-core, arboard) into the deliberately small server binary, and it inverts the current dependency direction. |
| One source file `#[path]`-included by both crates | Rejected. `snip-sync` is published, and `cargo package` cannot include a file outside the package root, so the server's packaging step (`scripts/release-check.sh:173-176`) would fail. |

### 3.5 A conflict between the roadmap and the M001 closure record

The roadmap assigns the `inspect`/`worker_inspect` and `is_stale`/
`worker_is_stale` collapse to **M002**
(`plans/subsystems/locking-convergence-roadmap.md:265-267`); the M001 closure
record assigns the same self-duplication to **M003**
(`plans/closure/locking-convergence/001-status.md`, "Deferred work" section).
This plan follows the roadmap, which is authoritative under
`plans/003-planning-process.md`. The scope is drawn so the milestones do not
overlap: **M002 collapses the duplicate implementations to one each while
keeping every public name**; **M003 deletes the `WorkerLock` tier and its
names** (§5, §16).

## 4. Invariants that must not regress

Drawn from `AGENTS.md` and `architecture/overview.md` §7 ("Locks are
kernel-backed and ordered", `architecture/overview.md:476-496`).

1. **The kernel is the authority.** A lock file's presence proves nothing;
   only the kernel lock state does (`src/process_file_lock.rs:11-15`).
2. **The kernel-backed family releases on `Drop` without unlinking.** Lock
   files may hold stale metadata (`architecture/overview.md:484-486`;
   `src/process_file_lock.rs:190-200`); the `snip-sync` copies already honor
   this (`snip-sync/src/server_lock.rs:121-129`,
   `snip-sync/src/process.rs:188-211`, `remove_on_drop: false` at
   `snip-sync/src/process.rs:233` and `:284`).
3. **Identity metadata never authorizes stealing.** A busy kernel lock with
   malformed, empty, or legacy metadata is still a busy lock
   (`src/process_file_lock.rs:21-24`).
4. **Unsupported platforms fail loudly** rather than weakening exclusion
   (`src/process_file_lock.rs:346-350`).
5. **`kill(pid, 0)`: only `ESRCH` proves absence.** `EPERM` and unknown
   errnos mean live (`src/utils/process.rs:29`). Linux start tokens use
   `/proc/<pid>/stat` field 22 (`src/process_file_lock.rs:500-504`).
6. **The mutation gate and journals are untouched.**
   `gate_mutation_on_interrupted_transactions()` runs before every local
   mutation; journals live in `<config>/.transaction/`
   (`architecture/overview.md:446-455`).
7. **The owned-record family is untouched.** `src/record_lock.rs:458` owns the
   `create_new` + ownership-record protocol; the transaction lock keeps its
   fail-fast policy (`src/transaction.rs:962`) and the local-data lock its
   wait-for-progress policy (`src/local_data.rs:131`).
8. **The Windows locked range stays a single byte at `u32::MAX`**
   (`src/process_file_lock.rs:230`, `:236`), and the Windows MSVC
   `/STACK:8388608` link flag stays (`.cargo/config.toml`).
9. **No new syscall on the uncontended path**
   (`plans/subsystems/locking-convergence-roadmap.md:364-367`).
10. **No credential or sync metadata in a lock payload**
    (`src/process_file_lock.rs:645-670`).

## 5. Scope

### In scope

- One shared workspace crate holding the kernel-lock primitive and the one
  busy-versus-error classification.
- `src/process_file_lock.rs` rewritten to delegate the kernel step and the
  classification, keeping its public surface; `snip-sync/src/server_lock.rs`
  rewritten to delegate, keeping `ServerLock`, `ServerLockIdentity`,
  `ServerLockError`, `ServerLock::try_acquire`, `server_lock_path`, and
  `read_owner`.
- The `croncheck.lock` copy in `snip-sync/src/process.rs` deleted, and
  `snip-sync/src/main.rs:648` routed through the shared primitive.
- The `inspect`/`worker_inspect` and `is_stale`/`worker_is_stale` pairs in
  `src/auto_sync/execution_lock.rs` collapsed to one implementation each, and
  the redundant `process_alive` `cfg` split removed. Public names stay.
- The three `UnsupportedPlatform` → `ErrorKind::Unsupported` conversions
  collapsed to one shared conversion in the client crate.
- Static guards in `tests/architecture.rs` asserting that the platform
  primitive and the busy classification each exist in exactly one place.
- The documentation and release-process updates listed in §12.

### Explicitly out of scope

- **The owned-record lock family** (`src/record_lock.rs`,
  `src/transaction.rs`, `src/local_data.rs`). M001 owns it; it is closed.
- **The lock acquisition order.** That is M003
  (`plans/subsystems/locking-convergence-roadmap.md:288-303`).
- **Retiring the `WorkerLock` tier.** M003 owns the tier's removal from the
  code, the status surface, and the architecture documents
  (`plans/subsystems/locking-convergence-roadmap.md:300-303`). M002 only
  collapses the duplicated *helper bodies* and keeps every public name.
- **The `status --json` schema.** `src/status_snapshot.rs:15`
  (`SNAPSHOT_SCHEMA`) and the `execution.worker_lock` field are M003's
  concern (`docs/JSON_SCHEMAS.md:168-171`).
- **The `LockIdentity` and `ServerLockIdentity` on-disk formats.** Both stay
  byte-identical. No migration.
- **Any wire-protocol, CLI, or exit-code change**
  (`plans/subsystems/locking-convergence-roadmap.md:335-339`).
- **Lock fencing, lock authority changes, and the auto-sync/sync authority
  relationship** — ADR triggers that stop the milestone
  (`plans/subsystems/locking-convergence-roadmap.md:401-403`).

## 6. Required production changes

### 6.1 A new shared workspace crate

Create `snip-lock/` as a new workspace member. `snip-lock/Cargo.toml` — package
`snip-lock`, version `0.1.0`, edition 2024, `rust-version = "1.94"`, a
`[lib]` named `snip_lock`. It carries the same platform dependencies the two
consumers already carry: `libc` on non-Windows (`Cargo.toml:118`),
`windows-sys` with `Win32_Storage_FileSystem` on Windows (`Cargo.toml:113`).
It carries **no** other dependency — no `serde`, no `toml`, no `uuid` — so it
can hold the *primitive* without owning the metadata format.

`snip-lock/src/lib.rs` — the whole crate, owning:

- `KernelLockError` with four variants: `Busy`, `Interrupted`, `Io`,
  `UnsupportedPlatform`.
- `KernelFileLock` — the RAII guard over the open file and the kernel lock.
  `Drop` releases the kernel lock and closes the descriptor; it **never
  unlinks** the lock file.
- `try_lock_kernel(path) -> Result<Option<KernelFileLock>, KernelLockError>` —
  opens the file, takes the lock non-blocking, returns `Ok(None)` for
  contention so each caller renders "busy" in its own vocabulary.
- `wait_lock_kernel(path, timeout)` — polls every 100 ms to the deadline,
  matching the canonical client's poll interval
  (`src/process_file_lock.rs:257`).
- `classify_unix_errno(raw: Option<i32>) -> KernelLockError` and
  `classify_windows_code(raw: Option<i32>) -> KernelLockError` — the single
  busy-versus-error definition.
- The Unix `flock` arm and the Windows `LockFileEx` arm, each exactly once.
- `LOCKED_BYTE_COUNT = 1` and `LOCKED_BYTE_OFFSET = u32::MAX`, moved from the
  three copies listed in §3.1.

### 6.2 The single busy-versus-error classification

| Condition | Outcome | Source of the decision |
|-----------|---------|------------------------|
| `EWOULDBLOCK` / `EAGAIN` | `Busy` | Unchanged everywhere. |
| `EACCES` | `Busy` | **Decision — the canonical client's classification wins** (`src/process_file_lock.rs:407`, pinned by the test at `:535`). Matches the roadmap's "most permissive" principle (`plans/subsystems/locking-convergence-roadmap.md:393-397`): contention is left to the caller's wait/retry policy rather than aborting the operation. The two `snip-sync` copies change from fatal to busy. |
| `EINTR` | `Interrupted` | **Decision — retried**, matching the server mirror's existing behavior (`snip-sync/src/server_lock.rs:189-191`, `:213-214`) and the roadmap's decision that "unification adopts the most permissive retry" (`plans/subsystems/locking-convergence-roadmap.md:393-397`). The canonical client changes from fatal to retried. |
| Any other errno | `Io` | Unchanged everywhere. |
| Windows raw `32` / `33` | `Busy` | Unchanged — all three copies already agree. |
| Any other Windows raw code | `Io` | Unchanged everywhere. |
| Non-Unix, non-Windows | `UnsupportedPlatform` | Unchanged (`src/process_file_lock.rs:346-350`). |

The two rows marked **Decision** are the behavior changes; §8.1 tabulates them
and §16 explains the alternatives. The `snip-sync` crate must not carry its
own `is_retryable_lock_error` (`snip-sync/src/server_lock.rs:189-196`) or its
own `WouldBlock` test (`snip-sync/src/process.rs:236`).

### 6.3 The three consumers become metadata layers over the primitive

| Module | Keeps | Moves out to `snip-lock` |
|--------|-------|--------------------------|
| `src/process_file_lock.rs` | `ProcessFileLock`, `LockIdentity` (`:64-80`), `ProcessFileLockError`, `try_acquire`, `wait_acquire`, `read_owner`, `read_identity_via_handle`, `nonce`, `path`; `publish_identity` (`:410-418`) with the `0o600` tightening (`:363-385`) and the "publication failure releases the lock" rule (`:390-396`); `current_start_token` (`:431`); the 100 ms poll interval and deadline semantics of `wait_acquire` (`:251-271`) | the `flock` / `LockFileEx` calls; `is_lock_busy_error`; the release helpers; the Windows range constants |
| `snip-sync/src/server_lock.rs` | `ServerLock`, `ServerLockIdentity` (`:34-47`) and its on-disk format **unchanged**, `ServerLockError` (`:56-66`), `ServerLock::try_acquire`, `server_lock_path`, `read_owner` and the read-only probe (`snip-sync/src/startup.rs:925-931`); the legacy-PID-file owner fallback (`:50-52`) | `acquire_unix_lock`; `is_retryable_lock_error`; the `LockFileEx` block; the release helpers; the range constants |
| `snip-sync/src/process.rs` | nothing — the whole module is retired | `try_lock`; the `LockGuard` type and its `Drop` impl; the range constants |

The "moves out" column names the items listed in §3.1 for each module. Two
consequences follow from the first row. The client's public surface is
unchanged, so `src/auto_sync/execution_lock.rs`,
`src/auto_sync/pending_lock.rs`, `src/status_snapshot.rs`, and
`tests/mutual_exclusion.rs` need no signature changes. And
`parse_linux_proc_start_token` (`:500-504`) stays duplicated with
`src/record_lock.rs:101-116` — `architecture/process_file_lock.md:297-300`
records that duplication for the owned-record family, this milestone does
**not** merge the two families, and the static guard must not claim
otherwise. `ProcessFileLockError` keeps its current shape: the shared
`Interrupted` outcome is retried inside `snip-lock` and never reaches the
client, so no new variant is needed.

`snip-sync/src/main.rs:647-648` is routed through the shared primitive with
the call site's semantics preserved exactly: `Some(lock)` means "I hold it",
`None` means "another croncheck is running; skip this check"
(`snip-sync/src/main.rs:650-652`), an error is a hard failure. The
`croncheck.lock` file keeps its name, location, and never-unlinked behavior.

### 6.4 The client's self-duplication is collapsed

In `src/auto_sync/execution_lock.rs`: `worker_inspect` (`:283-285`) becomes a
one-line delegation to `inspect` (`:166-168`); `worker_is_stale` (`:287-289`)
becomes a one-line delegation to `is_stale` (`:171-173`); the redundant
`#[cfg(unix)]` / `#[cfg(not(unix))]` split of `process_alive` (`:179-182`,
`:188-191`) is replaced by one unconditional definition; and the
`UnsupportedPlatform` → `ErrorKind::Unsupported` conversion is defined once
in the client crate and reused by `src/auto_sync/execution_lock.rs:95-98`,
`:237-240`, and `src/auto_sync/pending_lock.rs:64-67`. All public names are
retained. M003 removes the `WorkerLock` tier and its names; M002 removes no
name.

## 7. Ordered work packages

### Work package A — Create the shared crate and move the primitive

**Required changes**

1. Create `snip-lock/Cargo.toml` and `snip-lock/src/lib.rs` per §6.1–§6.2.
2. Add `"snip-lock"` to the root `Cargo.toml` `[workspace] members`
   (`Cargo.toml:180-182`), and `snip-lock = { version = "0.1.0", path =
   "snip-lock" }` to `Cargo.toml`'s and `snip-sync/Cargo.toml`'s
   `[dependencies]`.
3. Add `"snip-lock/"` to the `exclude` list in `Cargo.toml` (the list at
   `Cargo.toml:14-44` already excludes `snip-proto/` and `snip-sync/`, so the
   path dependency is not vendored into the published `snip-it` package).
4. Move the platform primitive and the classification out of
   `src/process_file_lock.rs` into `snip-lock`. Update `Cargo.lock`.

**Acceptance evidence**

- `cargo build -p snip-lock` succeeds on Linux, and `cargo tree -p snip-lock`
  shows no dependency other than `libc` / `windows-sys`.
- `grep -rn "libc::flock" src/ snip-sync/src/ snip-lock/src/` reports exactly
  one hit, in `snip-lock/src/lib.rs`; the same for `LockFileEx`.

### Work package B — Delegate `src/process_file_lock.rs` to the shared crate

**Required changes**

1. Rewrite `src/process_file_lock.rs` per §6.3: the module keeps its public
   surface and its metadata behavior and delegates the kernel step and the
   classification.
2. Keep `src/lib.rs:42`'s `pub mod process_file_lock;` export unchanged, so
   `src/auto_sync/execution_lock.rs`, `src/auto_sync/pending_lock.rs`,
   `src/status_snapshot.rs`, and `tests/mutual_exclusion.rs` need no change.
3. Keep the `LockIdentity` on-disk format byte-identical; the existing
   round-trip and cross-version parse tests must stay green.

**Acceptance evidence**

- `cargo test --workspace --lib` is green, including
  `eagain_and_ewouldblock_are_lock_contention`
  (`src/process_file_lock.rs:530-537`).
- `cargo test --test mutual_exclusion` and `cargo test --test
  process_lock_concurrency --features test-support -- --test-threads=1` are
  green (real subprocess contenders, real `flock`). A lock acquired through
  the delegated path still parses with the previous `LockIdentity` shape.

### Work package C — Delegate `snip-sync/src/server_lock.rs` to the shared crate

**Required changes**

1. Rewrite `snip-sync/src/server_lock.rs` per §6.3, deleting
   `is_retryable_lock_error` (`:189-196`) and `acquire_unix_lock`
   (`:205-221`).
2. Preserve `ServerLockIdentity`'s on-disk format, the legacy-PID-file owner
   fallback, and the `Busy { owner: None }` distinction that
   `snip-sync/src/main.rs:398` relies on.

**Acceptance evidence**

- `cargo test -p snip-sync --lib` is green, including the server mirror's
  `retryable_lock_error_is_only_eintr` (`:667-678`), which must be
  **rewritten** to assert the unified classification rather than the old
  mirror-local one.
- `cargo test -p snip-sync --test server_findings` is green: it spawns a real
  child process that holds the server lock and asserts the parent sees
  `Busy { owner }` with the holder's PID
  (`snip-sync/tests/server_findings.rs:146-156`).
- `cargo test --test snip_sync_lifetime -- --test-threads=1` is green — the
  wire-parity suite must be unchanged (`AGENTS.md` calls this suite out as
  locked).

### Work package D — Delete the `croncheck.lock` copy

**Required changes**

1. Delete `try_lock`, `LockGuard`, its `Drop` impl, and the range constants
   from `snip-sync/src/process.rs` (§6.3).
2. Route `snip-sync/src/main.rs:648` through the shared primitive, preserving
   the `Some` / `None` / `Err` semantics. Keep the `croncheck.lock` file name
   and location unchanged.

**Acceptance evidence**

- `grep -rn "fn try_lock" snip-sync/src/` reports no remaining definition,
  and `grep -rn "flock\|LockFileEx" snip-sync/src/process.rs` reports nothing.
- `cargo test -p snip-sync --lib` is green, including
  `stale_on_disk_lock_file_does_not_block_kernel_lock`
  (`snip-sync/src/process.rs:457-465` in the current tree), rewritten to
  exercise the shared primitive: a stale `croncheck.lock` on disk must not
  block acquisition. The croncheck path's "another croncheck is running;
  skip" behavior is still covered.

### Work package E — Collapse the client's self-duplication

**Required changes**

1. Collapse `worker_inspect` → `inspect` and `worker_is_stale` → `is_stale`
   in `src/auto_sync/execution_lock.rs` (§6.4), keeping both public names.
2. Remove the redundant `process_alive` `cfg` split.
3. Define the `UnsupportedPlatform` → `ErrorKind::Unsupported` conversion
   once and reuse it from `src/auto_sync/execution_lock.rs:95-98`,
   `:237-240`, and `src/auto_sync/pending_lock.rs:64-67`.

**Acceptance evidence**

- `grep -c "process_file_lock::read_owner" src/auto_sync/execution_lock.rs`
  reports `1`; `grep -rn "fn process_alive" src/auto_sync/execution_lock.rs`
  reports exactly one definition; `grep -rn "process file lock is not
  supported on this platform" src/` reports exactly one definition site.
- `cargo test --workspace --lib` is green, including
  `src/auto_sync/execution_lock.rs`'s unit tests and
  `src/auto_sync/pending_lock.rs`'s tests (`:121-245`).

### Work package F — Add the static guards

**Required changes**

Add two guards to `tests/architecture.rs`, following the pattern M001
established there (the existing three guards are at `tests/architecture.rs:425`,
`:438`, and `:478`):

1. **`kernel_lock_platform_primitive_exists_in_exactly_one_crate`** — scans
   every `.rs` file under `src/`, `snip-sync/src/`, and `snip-lock/src/` and
   asserts the platform primitive (`libc::flock` and `LockFileEx`) is defined
   in exactly one file, and that the file is `snip-lock/src/lib.rs`.
2. **`kernel_lock_busy_classification_exists_in_exactly_one_place`** — scans
   the same tree and asserts the busy classification is defined in exactly
   one place, and that the mirror-local `is_retryable_lock_error` and the
   `WouldBlock`-kind test are gone.

Each guard must be paired with a **synthetic-fixture test** proving the
detector can fail, following
`owned_record_lock_protocol_detector_reports_a_second_copy`
(`tests/architecture.rs:438-473`). A guard that cannot fail proves nothing.

**Acceptance evidence**

- `cargo test --test architecture` is green, and the synthetic-fixture tests
  demonstrate that a second copy of the primitive, and a second definition of
  the classification, are each detected.
- `tests/architecture.rs` still carries no module-level
  `#![allow(dead_code)]` (the existing guard at `tests/architecture.rs:247`
  enforces this over the whole tree).

### Work package G — Documentation and release-process updates

**Required changes**

Apply the §12 table in full.

**Acceptance evidence**

- `bash scripts/release-check.sh dry-run snip-lock` succeeds.
- `grep -rn "mirrored implementation" architecture/` reports nothing about
  the server singleton lock.
- Every `file:line` reference added to the architecture documents points at a
  line that exists in the tree after the change.

### Work package H — Full verification

**Required changes**

Run §11 in full. No code changes are expected from this package; any failure
is a defect in packages A–G.

**Acceptance evidence**

- §13's acceptance criteria are all satisfied, with the command output
  recorded.

## 8. Failure, cancellation, restart, and contention semantics

### 8.1 The unified classification

| Condition | Unified outcome | Client before | Server singleton before | `croncheck` before |
|-----------|-----------------|---------------|------------------------|-------------------|
| Lock held by another process | `Busy` | Busy | Busy | Busy |
| `EACCES` | `Busy` | Busy | **Io (fatal)** | **Io (fatal)** |
| `EINTR` | `Interrupted` → retried | **Io (fatal)** | retried | **Io (fatal)** |
| Other errno / Windows code | `Io` | Io | Io | Io |
| Non-Unix, non-Windows | `UnsupportedPlatform` | UnsupportedPlatform | UnsupportedPlatform (never constructed on Unix) | `create_new` fallback (`snip-sync/src/process.rs:288-305`) |

The two behavior changes are the `EACCES` and `EINTR` rows. Both are
recorded in `CHANGELOG.md` and in the closure record, as the roadmap requires
("each caller's behavior change is recorded in the closure record",
`plans/subsystems/locking-convergence-roadmap.md:393-397`).

### 8.2 Contention is not corruption

A `Busy` outcome is not an error condition and must not be rendered as one.
The client keeps `ProcessFileLockError::Busy { owner }` and
`ExecutionLockError::AlreadyHeld` (`src/auto_sync/execution_lock.rs:72-83`);
the server keeps `ServerLockError::Busy { owner }`
(`snip-sync/src/server_lock.rs:58-61`) and the "already running" message
(`snip-sync/src/main.rs:99-107`); the `croncheck` path keeps "Another
croncheck is already running; skipping this check."
(`snip-sync/src/main.rs:650-652`).

### 8.3 `EINTR` retry does not spin forever

`EINTR` is retried inside `snip-lock`, bounded by the caller's own wait
policy. `try_lock_kernel` retries a bounded number of times and then reports
the interruption as an `Io`-class failure — a pathological signal storm must
not produce an infinite loop. `wait_lock_kernel` retries within the caller's
deadline, so the 100 ms poll interval and the deadline semantics of
`src/process_file_lock.rs:251-271` are preserved.

### 8.4 Restart, crash, and the uncontended path are unchanged

The kernel releases the lock when the owning process dies. A new acquirer
succeeds on the first attempt; there is no stale-PID classification step in
this family (`src/process_file_lock.rs:11-15`). The `snip-sync` copies
already behave this way, and the shared primitive must preserve it: the lock
file is **not** unlinked on release, and a stale file on disk does not block
acquisition (the current `croncheck` test at
`snip-sync/src/process.rs:457-465` pins this). The uncontended acquisition
sequence stays: open → one `flock` / `LockFileEx` → (client only) publish
the identity. The shared crate must not add a probe, a stat, or an extra open
to that path.

## 9. Compatibility and migration

- **No schema migration.** Lock file names, directories, and payload formats
  are unchanged, so a lock taken by an older build stays recognizable to a
  newer one and vice versa
  (`plans/subsystems/locking-convergence-roadmap.md:329-331`).
- **No wire-protocol, CLI, exit-code, or JSON schema change.** The server's
  HTTP and gRPC surfaces are untouched
  (`plans/subsystems/locking-convergence-roadmap.md:337-338`).
  `SNAPSHOT_SCHEMA` stays `1` (`src/status_snapshot.rs:15`) and the
  `execution.worker_lock` field stays (`src/status_snapshot.rs:119`) — that
  removal is M003's work.
- **Lock files written by an older build still parse** and do not block a
  newer one: the client's `LockIdentity` and the server's
  `ServerLockIdentity` keep their fields and their `schema_version`, and the
  kernel lock state — not the file's bytes — is authoritative.
- **The new crate is a release-process change, not a migration.** `snip-lock`
  is published at `0.1.0` and enters the manual publish order
  (`RELEASING.md`, `architecture/overview.md:128-130`). Nothing on disk
  changes, and the `croncheck.lock` file keeps its name — a `croncheck.lock`
  left on disk by an older build is ignored by the newer build exactly as
  before.

## 10. Required tests

### 10.1 New unit tests in `snip-lock`

- `classify_unix_errno` maps `EWOULDBLOCK`, `EAGAIN`, and `EACCES` to
  `Busy`; maps `EINTR` to `Interrupted`; maps every other errno to `Io`.
  This is the single definition the roadmap's verification strategy calls for
  ("a test that pins the busy-versus-error classification for each platform
  error kind", `plans/subsystems/locking-convergence-roadmap.md:380-381`).
- `classify_windows_code` maps raw `32` and `33` to `Busy` and everything
  else to `Io`.
- `try_lock_kernel` on a held lock returns `Ok(None)`; on a free lock it
  returns `Ok(Some(guard))`, and the guard releases on `Drop` so a second
  acquisition succeeds.
- A stale lock file on disk does not block acquisition, and the lock file
  still exists after release (the kernel family never unlinks).

### 10.2 Rewritten tests

- `snip-sync/src/server_lock.rs:667-678`'s `retryable_lock_error_is_only_eintr`
  — the helper it tests is deleted. Replace it with a test that asserts the
  **unified** classification through the server's public API.
- `snip-sync/src/process.rs:457-465`'s
  `stale_on_disk_lock_file_does_not_block_kernel_lock` — move the equivalent
  assertion onto the shared primitive.
- `src/process_file_lock.rs:530-537`'s
  `eagain_and_ewouldblock_are_lock_contention` — the local
  `is_lock_busy_error` is deleted. Replace it with a test that asserts the
  client's behavior through the shared classification, keeping the same
  assertions (`EACCES` busy, `EINTR` not busy) so the classification's
  direction stays pinned.

### 10.3 Existing suites that must stay green

- `tests/process_lock_concurrency.rs` — real subprocess contenders (`:68`,
  `:125`, `:152`, `:180`). Serial.
- `tests/auto_sync_concurrency.rs` — including
  `test_symlink_substitution_on_worker_lock_resisted` (`:793`).
- `tests/mutual_exclusion.rs` — including
  `test_stale_lock_with_dead_pid_is_recovered` (`:143`).
- `snip-sync/tests/server_findings.rs` — including the child-process holder
  scenario (`:146-156`).
- `tests/snip_sync_lifetime.rs` — the wire-parity suite.
- `tests/recovery_integration.rs` — including
  `test_sync_repair_apply_preserves_persistent_locks` (`:481`).

## 11. Required verification commands

Run these in order. **Never run a cargo command while another test run is in
flight** — the integration tests execute `target/debug/snp`, and a rebuild
replaces that binary mid-run and produces false failures (`AGENTS.md`,
"Gotchas that break builds or corrupt state").

```bash
# 0. Formatting and lint (no test is running)
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# 1. Lint the test-support-gated targets that scripts/check.sh does NOT lint.
#    check.sh does not pass --all-features, so those targets are never linted
#    by it; lint them locally before landing (AGENTS.md).
cargo clippy --workspace --all-targets --all-features -- -D warnings

# 2. Unit tests (parallel-safe; each test uses an isolated TempDir)
cargo test --workspace --lib

# 3. The new crate's own tests
cargo test -p snip-lock

# 4. Static guards
cargo test --test architecture

# 5. Serial contention suites
cargo test --test process_lock_concurrency --features test-support -- --test-threads=1
cargo test --test auto_sync_concurrency --features test-support -- --test-threads=1
cargo test --test mutual_exclusion --features test-support -- --test-threads=1

# 6. Server-side lock suites
cargo test -p snip-sync --lib
cargo test -p snip-sync --test server_findings
cargo build -p snip-sync --bin snip-sync
cargo test --test snip_sync_lifetime -- --test-threads=1

# 7. The authoritative Linux CI gate
bash scripts/check.sh

# 8. Production seam proof (no new seam is introduced, but the primitive
#    moves and the seam binary must not gain one)
bash scripts/ci/test-production-seams.sh
```

`scripts/check.sh` is the authoritative Linux CI gate (`AGENTS.md`); it
already runs `cargo fmt`, `cargo clippy --workspace --all-targets`,
`cargo test --workspace --lib`, the serial `auto_sync_concurrency` and
`sync_multibatch` targets, and the `snip_sync_lifetime` suite.
`cleanup_crash_failpoints` and `restore_crash_failpoints` are invoked by **no
script** and must be run by hand before merging a restore or cleanup change
— this milestone changes neither, so they are not required here, but run them
if the change touches `src/transaction.rs`, `src/local_data.rs`, or
`src/record_lock.rs` at all. `bash scripts/release-check.sh dry-run snip-lock`
and `dry-run snip-sync` must succeed after work package G.

## 12. Documentation updates

| Document | Change |
|----------|--------|
| `architecture/process_file_lock.md` | The primitive's new home (`snip-lock`); the client and server both delegate; the `croncheck` copy is gone; the classification is defined once. Update the module inventory (`:22-36`), "Call Sites" (`:319-369`), and "Lock Family Boundary" (`:373-473`); remove the "mirrored implementation" wording at `:351-353`. |
| `architecture/overview.md` | §3 "Workspace Crates" (`:120-127`) gains `snip-lock` and the new publish order (`:128-130`); §7 "Locks are kernel-backed and ordered" (`:476-496`) names the shared crate. |
| `architecture/auto_sync.md` | The module-map rows for `execution_lock.rs` (`:61`) and `lock.rs` (`:62`). |
| `architecture/server.md` | The server's singleton lock delegates to the shared crate. |
| `.skills/transactions-and-auto-sync.md` | "Lock Hierarchy & Process Locks" (`:52-68`) names the shared crate and keeps the two-family distinction. |
| `docs/LOGICAL_LAYERS.md` | A layer row for the new crate, **together with** the `tests/architecture.rs` change, per that document's own instruction. |
| `RELEASING.md` | The crate table (`:18-24`) and the publish steps (`:135-160`) gain `snip-lock`; the order becomes `snip-lock` → `snip-proto` → `snip-sync` → `snip-it`. |
| `scripts/release-check.sh` | The `dry-run` allowlist (`:187-193`) and the package list (`:172-176`) gain `snip-lock`. |
| `CHANGELOG.md` | One entry for the convergence, naming the `EINTR` and `EACCES` behavior changes. |
| `AGENTS.md` | Only if the "Layout" section's crate list or the "Gotchas" table needs a new row for the shared crate. |

## 13. Acceptance criteria

1. Exactly one implementation of each platform's kernel-lock primitive exists
   in the workspace: `libc::flock` and `LockFileEx` each appear in exactly
   one file, and that file is `snip-lock/src/lib.rs`.
2. The busy-versus-error classification is defined once and referenced by
   every caller. The mirror-local `is_retryable_lock_error` and the
   `WouldBlock`-kind test are gone.
3. `snip-sync/src/process.rs` no longer contains a lock implementation; the
   `croncheck.lock` path goes through the shared primitive and keeps its
   "another croncheck is running; skip" behavior and its lock file name.
4. `inspect`/`worker_inspect` and `is_stale`/`worker_is_stale` each have one
   implementation, and every public name is still present.
5. The `UnsupportedPlatform` → `ErrorKind::Unsupported` conversion exists once
   in the client crate.
6. `tests/architecture.rs` contains a guard for (1) and (2), and each guard
   has a synthetic-fixture test proving it can fail.
7. The `LockIdentity` and `ServerLockIdentity` on-disk formats are
   byte-identical to the baseline; no lock file needs migrating.
8. `SNAPSHOT_SCHEMA` is still `1` and the `execution.worker_lock` field is
   still present — M003's work, not this milestone's.
9. `src/record_lock.rs`, `src/transaction.rs`, and `src/local_data.rs` are
   unchanged: the owned-record family is untouched.
10. Every command in §11 passes, and `bash scripts/check.sh` is green.
11. The `EACCES` and `EINTR` behavior changes are recorded in `CHANGELOG.md`
    and in the closure record.
12. `snip-lock` is publishable: `bash scripts/release-check.sh dry-run
    snip-lock` succeeds.

## 14. Stop conditions

Stop the milestone and escalate rather than proceeding if any of these hold.

1. **The shared primitive cannot be placed where both crates can reach it.**
   If a new published crate is rejected (release-process policy, crates.io
   naming, versioning constraint) and the alternatives — `snip-sync`
   depending on `snip-it`, or a `#[path]`-included source file — are also
   rejected, the exit condition "exactly one implementation of each platform's
   lock primitive exists in the workspace" cannot be met and the milestone
   stops.
2. **The classification change would alter lock authority.** If unifying
   `EACCES` or `EINTR` requires changing *which* process may hold a lock, or
   requires a fencing token, the milestone becomes an ADR trigger
   (`plans/subsystems/locking-convergence-roadmap.md:401-403`) and stops.
3. **A Windows `LockFileEx` behavior cannot be verified on this platform.**
   The Windows arm's busy codes (`32`, `33`), the locked byte range, and the
   `UnlockFileEx` release cannot be exercised on Linux. If the change alters
   any of them, the milestone stops until it can be validated on Windows or on
   a Windows CI run — the change must not be asserted green on the strength
   of a source review alone.
4. **The server's `Busy` diagnostic would become misleading.** If making
   `EACCES` busy means the server can no longer distinguish "another server
   holds the lock" from "the state directory is not writable" *and* the
   distinction cannot be recovered from the diagnostic, the milestone stops
   until the diagnostic is reworked. The `Busy { owner: None }` arm
   (`snip-sync/src/main.rs:397-399`) is the existing mechanism and must keep
   working.
5. **The shared primitive would add a syscall to the uncontended path**, or
   would require touching the owned-record family (`src/record_lock.rs`,
   `src/transaction.rs`, `src/local_data.rs`) — M001's closed territory
   (`plans/subsystems/locking-convergence-roadmap.md:364-367`).
6. **`snip-lock` cannot be published.** If `cargo package -p snip-lock` or
   `bash scripts/release-check.sh dry-run snip-lock` cannot be made to
   succeed (crate name already taken on crates.io, packaging constraints,
   release-policy refusal), the milestone stops.
7. **The behavior changes cannot be recorded.** If the `EINTR` or `EACCES`
   change cannot be documented in `CHANGELOG.md` and the closure record, the
   milestone stops
   (`plans/subsystems/locking-convergence-roadmap.md:393-397`).

## 15. Closure evidence required

The closure record is written at
`plans/closure/locking-convergence/002-status.md` using the template in
`plans/closure/README.md`. It must contain:

1. The verification command list from §11 with the actual output, including
   `bash scripts/check.sh`.
2. The `grep` output proving one `flock` site, one `LockFileEx` site, and
   one busy-classification site.
3. The `tests/architecture.rs` guard names and the synthetic-fixture test
   results.
4. An explicit statement of the two behavior changes — `EINTR` is now
   retried on the client; `EACCES` is now busy on the server — with the
   before/after classification table from §8.1.
5. Evidence that the `LockIdentity` and `ServerLockIdentity` on-disk formats
   are unchanged (the cross-version parse tests, and a diff of the record
   definition).
6. The publish-order change and the `dry-run` results for `snip-lock` and
   `snip-sync`.
7. A statement that the owned-record family (`src/record_lock.rs`) was not
   modified in this milestone.
8. A list of every architecture document updated, with the `file:line`
   references that changed.

## 16. Handoff notes

- **M002 and M003 do not overlap.** M002 collapses the *helper
  implementations* in `src/auto_sync/execution_lock.rs` and keeps every public
  name. M003 deletes the `WorkerLock` tier, its names (`WorkerLock`,
  `WorkerLockError`, `try_acquire_worker`, `wait_acquire_worker`,
  `worker_inspect`, `worker_is_stale`, `worker_lock_path`,
  `WORKER_LOCK_NAME`, `WORKER_LOCK_PURPOSE`), the `src/auto_sync/lock.rs`
  shim, the `status_snapshot.rs` worker-lock inspection, and the
  `execution.worker_lock` JSON field. If M002 lands first, M003 finds M002's
  collapsed helpers already in place; if M003 lands first, M002's work package
  E shrinks to the `process_alive` split and the error-conversion collapse.
- **The roadmap/closure-record conflict on the self-duplication is unresolved
  upstream** (§3.5). This plan follows the roadmap (M002). If the parent
  decides the closure record is authoritative, move work package E's first two
  items to M003 and drop them here; nothing else changes.
- **The two classification directions are decisions, not discoveries.** §6.2
  states them: `EACCES` → `Busy`, matching the canonical client's existing
  production behavior (`src/process_file_lock.rs:407`, pinned by the test at
  `:535`) and the roadmap's "most permissive" principle; and `EINTR` →
  retried, forced by the roadmap's "unification adopts the most permissive
  retry" (`plans/subsystems/locking-convergence-roadmap.md:393-397`). The
  alternatives (`EACCES` → fatal everywhere, `EINTR` → fatal everywhere) would
  change the *client's* production behavior instead of the server's and would
  contradict existing pinned tests. The plan states each decision and its
  alternative so a reviewer can reverse it cheaply, but the plan as written is
  executable.
- **The new crate is the largest structural change in this milestone.** It
  touches `Cargo.toml`, `snip-sync/Cargo.toml`, `Cargo.lock`, `RELEASING.md`,
  and `scripts/release-check.sh`. If the parent prefers a different crate name
  or publication policy, that decision must be made before work package A
  starts, because the crate name appears in the publish order, the `dry-run`
  allowlist, and the architecture documents.
- **Do not run cargo concurrently with another agent's test run.** The
  integration tests execute `target/debug/snp`; a concurrent rebuild replaces
  that binary mid-run and produces false failures.
- **The `croncheck.lock` path is the smallest-surface change** but it is the
  one with no architecture documentation today, so the §12 update for
  `architecture/process_file_lock.md` adds a row that did not exist.
