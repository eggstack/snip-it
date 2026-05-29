# Clipboard Module Review

## Document Accuracy

### Verified Correct

| Claim (architecture/clipboard.md) | Source |
|---|---|
| File is `src/clipboard.rs` (162 lines) | Confirmed — 162 lines |
| Windows uses `clipboard-win` crate | `src/clipboard.rs:15` — `use clipboard_win::{formats, Clipboard}` |
| macOS/Linux uses `copypasta` crate | `src/clipboard.rs:18` — `use copypasta::{ClipboardContext, ClipboardProvider}` |
| Five public functions documented | All verified: `copy_to_clipboard` (line 91/107), `copy_to_clipboard_auto` (line 84), `copy_to_clipboard_with_auto_clear` (line 73), `schedule_clipboard_clear` (line 25), `clear_clipboard` (line 69) |
| Auto-clear uses `AtomicBool` to prevent concurrent schedules | `src/clipboard.rs:23` — `static CLIPBOARD_CLEAR_SCHEDULED: AtomicBool` confirmed |
| `copy_to_clipboard` is platform-conditional | `#[cfg(windows)]` at line 90, `#[cfg(not(windows))]` at line 106 — confirmed |
| Integration points: run_cmd, clip_cmd, cron_cmd, ui | Verified: `run_cmd.rs:84`, `clip_cmd.rs:20`, `cron_cmd.rs:42`, `ui.rs:833,889,1031` |
| Tests cover empty string, normal text, unicode, multiline, special chars, long content | All six tests present at lines 122–161 |

### Discrepancies

| Claim | Reality | Severity |
|---|---|---|
| Auto-clear step 4: "Reset scheduling flag" | Flag is reset *after* clipboard clear, not as a separate step — order is: clear → reset. Documentation implies it's a distinct step | Minor — semantics are close enough, but wording is imprecise |
| Doc doesn't mention `clear_clipboard_impl` as internal helper | `clear_clipboard()` at line 69 is a trivial wrapper around `clear_clipboard_impl()` at line 45 — the indirection is unexplained | Documentation gap |
| Doc says `copypasta` with `default-features = false` | `Cargo.toml:15` confirms this — copypasta is compiled without X11/Wayland feature flags, which could silently disable clipboard on Linux | Potential issue — see Design Issues |

## Bugs & Issues

### Bug 1: Auto-clear race condition — new clipboard content cleared prematurely (High)
**Location**: `src/clipboard.rs:30-42`

```rust
if CLIPBOARD_CLEAR_SCHEDULED.swap(true, Ordering::SeqCst) {
    return; // <-- new schedule silently skipped
}
```

When the auto-clear timer is running and a user copies new content, the new `schedule_clipboard_clear` call is silently rejected because the flag is still `true`. The *old* thread then wakes up and clears the *new* content. There is no mechanism to cancel or restart the timer on a fresh copy.

Scenario:
1. Copy snippet A → 30s timer starts
2. At 20s, copy snippet B → schedule is skipped
3. At 30s, thread clears snippet B (just copied)

This is a logic error that causes silent data loss.

### Bug 2: `drop(handle)` discards JoinHandle — no error propagation (Low)
**Location**: `src/clipboard.rs:42`

```rust
std::mem::drop(handle);
```

The `JoinHandle` is immediately dropped, meaning if the spawned thread panics, it is silently ignored. While a panic in `clear_clipboard` is unlikely (error handling via `map_err`), it's still a defensive gap. Using `handle.join()` or `let _ = handle.join()` would at least catch panics.

### Bug 3: No validation of `clipboard_auto_clear_seconds` (Low)
**Location**: `src/clipboard.rs:25-26`, `src/config.rs:30`

The config accepts `Option<u32>` with no upper bound. A value of `u32::MAX` (4,294,967,295 seconds ≈ 136 years) would:
- Set the `AtomicBool` to `true` for ~136 years
- Block all future auto-clear schedules for the lifetime of the process
- The sleeping thread would effectively never wake up

This should be capped at a reasonable maximum (e.g., 86400 seconds / 24 hours).

### Bug 4: `clear_clipboard` logs success unconditionally (Medium)
**Location**: `src/clipboard.rs:65`

```rust
log_clipboard_operation("clear_clipboard", true);
```

`log_clipboard_operation` is called with `success=true` *before* the function returns `Ok(())`. However, the call is on the success path so this is technically correct. The real issue is that `clear_clipboard_impl` is a thin wrapper — the indirection through `clear_clipboard` adds nothing and could be removed, reducing the API surface.

## Design Issues

### Design 1: Auto-clear tied to sync settings (Medium)
**Location**: `src/clipboard.rs:84-87`

```rust
pub fn copy_to_clipboard_auto(text: &str) -> SnipResult<()> {
    let settings = crate::config::get_sync_settings();
    copy_to_clipboard_with_auto_clear(text, settings.clipboard_auto_clear_seconds)
}
```

`copy_to_clipboard_auto` reads `SyncSettings` to determine auto-clear behavior. This couples clipboard auto-clear to the sync subsystem. A user who doesn't use sync should still be able to configure clipboard auto-clear. The function name `copy_to_clipboard_auto` doesn't convey that it reads sync settings — it reads like "copy to clipboard automatically" rather than "copy with sync-configured auto-clear."

### Design 2: Global static state for scheduling (Medium)
**Location**: `src/clipboard.rs:23`

```rust
static CLIPBOARD_CLEAR_SCHEDULED: AtomicBool = AtomicBool::new(false);
```

The flag is process-global. If snp is invoked multiple times (e.g., `snp run foo` then `snp clip bar`), each invocation creates its own process with its own flag — so this is fine for the CLI. However, if the module is ever used in a server/long-running context, this would be a problem. The current design is acceptable for a CLI tool but should be noted.

### Design 3: Thread handle is fire-and-forget (Low)
**Location**: `src/clipboard.rs:34-42`

The spawned thread is detached. This means:
- No way to join/cancel the thread
- If the program exits, the thread is killed (acceptable for CLI)
- If the thread panics, the flag stays `true` forever, blocking future schedules

A `CancellationToken` or storing the handle would be more robust.

### Design 4: `copypasta` compiled without default features (High)
**Location**: `Cargo.toml:15`

```toml
copypasta = { version = "0.8", default-features = false }
```

`copypasta`'s default features include `x11-feature` and `wayland-feature`. Disabling them means on Linux, the crate may fail to compile or silently provide non-functional clipboard support. If the intent is to support Wayland-only or X11-only, the correct feature flags should be explicitly enabled. This could cause clipboard failures on Linux that are hard to diagnose.

## Security Concerns

### Security 1: No sanitization of clipboard content (Informational)
**Location**: `src/clipboard.rs:91-119`

The module copies arbitrary text to the system clipboard without sanitization. This is expected behavior (snippets can contain shell commands, API keys, etc.), but it means the system clipboard may hold sensitive data for the auto-clear duration. The auto-clear feature mitigates this, but users may not know it's enabled or how long the delay is.

### Security 2: API key in `SyncSettings` could be copied (Low)
**Location**: `src/config.rs:21`

If a user accidentally copies `sync.toml` content or a snippet that contains the API key, it will be on the clipboard. The auto-clear feature helps, but the default is `None` (no auto-clear). Consider defaulting to a short auto-clear (e.g., 30s) when the feature is not explicitly disabled.

## Performance Issues

### Performance 1: `get_sync_settings()` reads and parses TOML on every copy (Low)
**Location**: `src/clipboard.rs:84-86`

```rust
let settings = crate::config::get_sync_settings();
```

`get_sync_settings()` reads `sync.toml` from disk, parses TOML, and returns a `SyncSettings` struct on every call to `copy_to_clipboard_auto`. This is called for every snippet copy. While the file is small and parsing is fast, this is wasteful for a hot path. The settings should be cached or read once at startup.

### Performance 2: `text.to_owned()` allocation (Informational)
**Location**: `src/clipboard.rs:113`

```rust
ctx.set_contents(text.to_owned())
```

On macOS/Linux, the text is cloned into a `String` before being passed to `set_contents`. This is a minor allocation that could be avoided if `copypasta` accepted `&str`. Not actionable without upstream changes.

## Priority Ranking

| # | Issue | Severity | Impact | Effort |
|---|---|---|---|---|
| 1 | Auto-clear race condition clears new content | High | Silent data loss on rapid copies | Medium |
| 2 | `copypasta` default features disabled | High | Clipboard broken on Linux | Low |
| 3 | No upper bound on `auto_clear_seconds` | Low | AtomicBool stuck for 136 years | Low |
| 4 | Global static blocks future schedules on panic | Low | Flag stuck true if thread panics | Low |
| 5 | `get_sync_settings()` parsed on every copy | Low | Unnecessary disk I/O per copy | Low |
| 6 | `drop(handle)` ignores thread panics | Low | No panic propagation | Trivial |
| 7 | Thread handle is fire-and-forget | Low | No cancellation mechanism | Low |
| 8 | Auto-clear tied to sync settings | Medium | Coupling clipboard to sync subsystem | Medium |

## Recommendations

1. **Fix auto-clear race condition**: On each new `copy_to_clipboard_with_auto_clear`, cancel the existing timer (use `Arc<AtomicBool>` or a generation counter) and restart with the new delay. Alternatively, document the limitation clearly.

2. **Verify `copypasta` feature flags**: On Linux, either enable `x11-feature` or `wayland-feature` explicitly, or confirm that the `default-features = false` works for the target platform. Test on a headless Linux system.

3. **Cap `clipboard_auto_clear_seconds`**: Reject values above 86400 (24h) in config validation.

4. **Remove `clear_clipboard` wrapper**: `clear_clipboard()` is a trivial delegation to `clear_clipboard_impl()`. Inline it or rename `clear_clipboard_impl` to `clear_clipboard` and remove the wrapper.

5. **Cache sync settings**: Read `SyncSettings` once at startup (or lazily) instead of parsing TOML on every clipboard copy.

6. **Consider `handle.join()`**: At minimum, `let _ = handle.join();` would catch panics in the clear thread and reset the flag.

7. **Add integration tests**: The current test suite only tests `copy_to_clipboard`. Add tests for:
   - `schedule_clipboard_clear` with mocked time
   - Race condition behavior (concurrent schedules)
   - `copy_to_clipboard_auto` with mock sync settings
