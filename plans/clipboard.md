# Clipboard Module Improvement Plan

## Architecture Document Verification

### Claims from `architecture/clipboard.md`:

| Claim | Status | Notes |
|-------|--------|-------|
| `copy_to_clipboard(text)` | VERIFIED | Implemented for both Windows and Unix |
| `copy_to_clipboard_auto(text)` | VERIFIED | Reads from sync settings |
| `copy_to_clipboard_with_auto_clear(text, seconds)` | VERIFIED | Works as documented |
| `schedule_clipboard_clear(seconds)` | VERIFIED | Implemented with AtomicU64 |
| `clear_clipboard()` | VERIFIED | Implemented |
| Auto-clear via `AtomicBool` | **MISLEADING** | Actually uses `AtomicU64` as generation counter |
| Windows uses `clipboard-win` | VERIFIED | Confirmed in source |
| macOS/Linux uses `copypasta` | VERIFIED | Confirmed in source |
| Integration: `src/commands/run_cmd.rs` | VERIFIED | Line 82 |
| Integration: `src/commands/clip_cmd.rs` | VERIFIED | Line 20 |
| Integration: `src/commands/cron_cmd.rs` | VERIFIED | Line 49 |
| Integration: `src/ui.rs` | **WRONG** | Actually `src/ui/mod.rs` line 619, 675, 817 |

---

## Bugs Found

### 1. Race Condition in `schedule_clipboard_clear` (Critical)

**Location**: `src/clipboard.rs:30-39`

```rust
let gen = CLIPBOARD_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
```

`fetch_add` returns the **previous** value before incrementing. So if `CLIPBOARD_GENERATION` is 5:
- `fetch_add(1)` returns 5
- `gen` becomes 6
- The spawned thread loads `CLIPBOARD_GENERATION.load()` which is 6

This works correctly. **However**, the logic is **fragile**:
- If a second call happens before the thread reads, the first thread will see the new generation and skip clearing
- But if the second call's thread spawns **after** the first thread reads but **before** it checks, both threads may attempt to clear

**Problem**: The comparison `gen == CLIPBOARD_GENERATION.load()` is not atomic with the spawn. A thread could read 6, then before it checks, another call increments to 7, and now two threads think they should clear.

**Fix**: Use a single `AtomicBool` flag as documented, or use channel-based cancellation.

---

### 2. Visual Mode Clipboard Copy Missing Audit Log

**Location**: `src/ui/mod.rs:675`

When copying multiple snippets via visual mode (`V` then `y`):
```rust
let _ = clipboard::copy_to_clipboard_auto(&copy_text);
```

Unlike `run_cmd.rs:83` and `clip_cmd.rs:21`, this does **not** call `audit_log("copy", snippet)`. The operation is silently logged only at `debug` level in the clipboard module.

---

### 3. UI Error Suppression - No User Feedback

**Locations**: `src/ui/mod.rs:619, 675, 817`

All three UI clipboard operations use:
```rust
let _ = clipboard::copy_to_clipboard_auto(&cmd);
```

Failures are completely silent. The user sees no indication if clipboard copy fails. Unlike `cron_cmd.rs:51` which prints an error.

---

### 4. `std::mem::drop(handle)` is Redundant

**Location**: `src/clipboard.rs:42`

```rust
std::mem::drop(handle);
```

Dropping a `JoinHandle` merely allows the thread to continue in the background. The thread is already spawned and will continue regardless. This line has no effect and should be removed.

---

### 5. Auto-Clear Failure Uses Wrong Log Level

**Location**: `src/clipboard.rs:37`

```rust
tracing::debug!("Auto-clear clipboard failed: {}", e);
```

If the clipboard fails to auto-clear, this is a **functional failure** - the clipboard retains sensitive content longer than expected. This should be `warn` or `error`, not `debug`. The `debug` level is inappropriate for operational failures.

---

## Potential Improvements

### 1. Clipboard Timeout Protection

**Problem**: `Clipboard::new()` and `set_text()` on Windows, `ClipboardContext::new()` and `set_contents()` on Unix have no timeout. On a system under heavy load or with clipboard locks held by another application, these can hang indefinitely.

**Recommendation**: Add a timeout mechanism using `thread::timeout` or wrapper that fails gracefully.

---

### 2. Visual Feedback on UI Clipboard Operations

**Problem**: Users in TUI have no confirmation clipboard copy succeeded.

**Recommendation**: Add a status message similar to `cron_cmd.rs:50-51`:
```rust
Err(e) => eprintln!("Failed to copy to clipboard: {}", e),
```

---

### 3. Thread Leak on Rapid Scheduling

**Problem**: If `schedule_clipboard_clear(1)` is called 1000 times rapidly, 1000 threads are spawned, each sleeping for 1 second. Each thread checks `gen == CLIPBOARD_GENERATION.load()` and exits quickly, but all must be scheduled and join.

**Recommendation**: Use a single background thread with a `Mutex<Option<Instant>>` for the scheduled clear time, or use a channel-based approach.

---

### 4. No Clipboard Content Type Preservation

**Problem**: Only handles text (`String`). Does not support images, files, or rich content.

**Recommendation**: Document this limitation. Consider adding `copy_image_to_clipboard` variant if needed.

---

### 5. Missing Test Coverage

**Problem**: No tests for:
- `schedule_clipboard_clear`
- `clear_clipboard`
- `copy_to_clipboard_with_auto_clear`
- Error paths on Windows (clipboard locked, etc.)
- Concurrent scheduling

**Recommendation**: Add integration tests for auto-clear behavior.

---

## Discrepancy Summary

1. **Documentation error**: Says `src/ui.rs` but actual is `src/ui/mod.rs`
2. **Documentation says `AtomicBool`**: Actual implementation uses `AtomicU64` (generation counter)
3. **Documentation implies simple boolean**: The generation counter is more complex and has subtle race conditions

---

## Priority Actions

1. **High**: Fix `tracing::debug` on line 37 to `tracing::warn`
2. **High**: Add audit log call to visual mode clipboard copy in `ui/mod.rs`
3. **Medium**: Add user-visible error messages in UI clipboard failures
4. **Medium**: Add timeout wrapper for clipboard operations
5. **Low**: Remove redundant `std::mem::drop(handle)`
6. **Low**: Add missing unit tests for `schedule_clipboard_clear`