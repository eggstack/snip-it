# Plan 012: Sync retry policy deduplication

Status: complete

Depends on: Plans 008–011

## Objective

Remove duplicated gRPC retry/backoff control flow in the sync client while preserving all current transport, timeout, encryption, batching, pagination, and error semantics.

This is intentionally the last plan in the sequence because it is internal cleanup with lower user value than CLI/library/search consolidation.

## Audit findings

`src/sync.rs` currently has separate `retry_grpc!` and `retry_grpc_limited!` macros. They duplicate retry classification, exponential backoff, jitter calculation, warning logging, retry counters, and terminal error conversion; the limited form additionally enforces the optional automatic-sync deadline.

The sync module also owns substantial legitimate complexity—TLS, authentication metadata, encryption/decryption, batching, pagination, message limits, and premade-library RPCs. This plan must not expand into a transport rewrite.

## Scope

Expected files:

```text
src/sync.rs
src/error.rs only if existing typed timeout mapping needs a small helper
tests/**
```

## Non-goals

Do not:

- change retry counts, default delays, timeout defaults, or jitter range without a proven bug;
- adopt a retry dependency;
- add middleware/tower layers solely for retry;
- rewrite Tonic transport setup;
- change sync protocol messages;
- alter encryption, batching, pagination, or merge behavior;
- create a generalized async resilience framework;
- add circuit breakers, rate limiters, background queues, or telemetry systems.

## Execution steps

### 1. Characterize current retry semantics

Before refactoring, write down or test:

- retryable and non-retryable gRPC status codes;
- total attempt count;
- exponential delay progression and cap;
- jitter bounds;
- ordinary request timeout behavior;
- automatic-sync deadline behavior before an RPC;
- deadline expiration during an RPC;
- deadline expiration before/during backoff;
- terminal `SnipError` classification.

The refactor must preserve these semantics.

### 2. Replace duplicate macros with one narrow executor

Prefer one private async helper that accepts:

- operation name;
- retry configuration;
- optional `SyncRunLimits`/deadline;
- an async operation factory/closure that can be invoked per attempt.

If Rust borrowing around the Tonic client makes a helper materially more complex than the macros, a single macro with an optional limits branch is acceptable. The goal is one implementation of retry/backoff policy, not abstraction purity.

### 3. Keep transport-specific call sites readable

Each RPC call site should still clearly show which operation is being attempted and what request is sent. Do not hide all sync operations behind generic boxed futures or dynamic dispatch.

### 4. Preserve timeout layering

Automatic sync must remain bounded by its total invocation deadline in addition to per-request transport limits. Manual sync should retain its existing behavior.

Make sure sleeps cannot extend a limited invocation beyond its deadline when the current code would refuse the retry.

### 5. Add deterministic tests where practical

Avoid tests that sleep for production-scale durations. If needed, expose a private/test-only retry config or injectable timing seam to verify:

- non-retryable error gets one attempt;
- retryable error reaches configured attempt count;
- eventual success stops retries;
- limited mode refuses a retry when deadline cannot accommodate backoff;
- timeout maps to the existing timeout failure kind.

Do not add a production clock abstraction unless there is no simpler test seam.

### 6. Verify sync regression coverage

Run existing multi-batch, pagination, encryption-failure, timeout, and integration tests in addition to:

```bash
cargo check --workspace --all-targets
cargo test --workspace
bash scripts/check.sh
```

## Acceptance criteria

Plan 012 is complete only when:

1. Retryability classification exists in one implementation path.
2. Backoff/jitter/counter logic is no longer duplicated between ordinary and deadline-limited retries.
3. Existing retry counts, delays, timeout behavior, and error classifications remain unchanged unless a focused regression test demonstrates a pre-existing bug and the completion note documents it.
4. Manual sync remains unbounded by the auto-sync total deadline except for existing request timeouts.
5. Automatic sync still honors the total deadline before requests and backoff.
6. Sync batching, pagination, encryption, merge, and protocol behavior are untouched except for call-site adaptation.
7. No retry/resilience dependency or framework is added.
8. Focused retry tests are deterministic and fast.
9. Existing sync regression tests and `bash scripts/check.sh` pass.
10. `plans/README.md` is updated to mark the overall consolidation sequence complete when Plans 008–012 are complete.

## Handoff note

This should be a small deletion-oriented refactor. If the proposed helper introduces complex lifetime machinery, choose the simpler single-policy implementation instead. Do not trade duplicated retry code for harder-to-maintain generic async plumbing.