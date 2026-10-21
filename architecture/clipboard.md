# Clipboard

[← Back to Overview](overview.md)

## Overview

Cross-platform clipboard access (`src/clipboard.rs`, 260 lines). Plain-text
only, one OS backend per platform, every OS call bounded by a timeout, and an
optional scheduled auto-clear.

This doc covers the OS layer (`src/clipboard.rs`) and the one sanctioned
side-effect funnel (`src/commands/clip_cmd.rs:40`). It does **not** cover the
TUI event loop that triggers copies (see [tui.md](tui.md)), the variable
expansion that produces the copied text (see
[utils/variables.md](utils/variables.md)), or the audit-trail format (see
[logging.md](logging.md)).

There are **three** similarly named functions, and picking the wrong one
silently changes observable behaviour:

| Function | Location | Auto-clear | Audit | Usage index |
|----------|----------|-----------|-------|-------------|
| `clipboard::copy_to_clipboard(text)` | `src/clipboard.rs:177` / `:196` | no | no | no |
| `clipboard::copy_to_clipboard_auto(text)` | `src/clipboard.rs:166` | yes, if configured | no | no |
| `clip_cmd::copy_to_clipboard(snippet, cmd)` | `src/commands/clip_cmd.rs:40` | yes, if configured | yes | yes |

`AGENTS.md` names `clip_cmd::copy_to_clipboard` as the path every snippet copy
must take; only two call sites bypass it (see
[Direct call sites](#direct-call-sites)).

## Platform strategy

### Dependency gating (`Cargo.toml`)

| Target | Dependency | Location |
|--------|------------|----------|
| `cfg(windows)` | `clipboard-win = "5.4"` | `Cargo.toml:98` |
| `cfg(not(windows))` (macOS + Linux) | `arboard = { version = "3", default-features = false }` | `Cargo.toml:102` |

`default-features = false` on `arboard` is load-bearing. arboard's default
feature set is `["image-data"]`, and the optional `wayland-data-control`
feature is **separate** and also off. Consequences for this build:

- **No Wayland backend.** arboard's `mod wayland` is
  `#[cfg(feature = "wayland-data-control")]`, so it is compiled out entirely.
  The Linux backend is X11-only, via `x11rb`.
- **No image clipboard formats.** `arboard::set_image` is unavailable; the
  snip-it code never calls it, so this is informational only.

`clipboard-win` is pulled in with its default features; the `monitor` feature
(change notification) and `std` (error trait) are not enabled. snip-it only
uses `clipboard_win::set_clipboard` with the `Unicode` format, which needs
neither.

### Backend behaviour

| Platform | Write path | Notes |
|----------|------------|-------|
| Windows | `clipboard_win::set_clipboard(formats::Unicode, &text)` — `src/clipboard.rs:180` | Opens the clipboard with 10 attempts (`clipboard-win` `lib.rs:234`), yielding via `Sleep(0)` between attempts |
| macOS | `arboard::Clipboard::new()` + `set_text` — `src/clipboard.rs:199-207` | NSPasteboard; content persists in the system pasteboard after the process exits |
| Linux | `arboard::Clipboard::new()` + `set_text` — `src/clipboard.rs:199-207` | X11 selection only (see above) |

Only plain text is supported: neither backend exposes rich-text/HTML formats
through the API snip-it uses, so content type cannot be preserved for
non-text payloads (`src/clipboard.rs:171-175`, `:190-194`).

## The three copy paths in detail

### `clipboard::copy_to_clipboard(text)` — raw OS write

```rust
// src/clipboard.rs:196
pub fn copy_to_clipboard(text: &str) -> SnipResult<()> {
    let text = text.to_owned();
    with_clipboard_timeout("copy", move || {
        let mut ctx = Clipboard::new().map_err(|e| {
            log_clipboard_operation("create context", false);
            SnipError::clipboard_error("create clipboard context", format!("{e}"))
        })?;

        ctx.set_text(&text).map_err(|e| {
            log_clipboard_operation("set_text", false);
            SnipError::clipboard_error("set text", format!("{e}"))
        })?;

        log_clipboard_operation("set_text", true);
        Ok(())
    })
}
```

Returns `SnipResult<()>`; on success the OS clipboard holds `text`. It
schedules **no** auto-clear and records **no** audit entry. The Windows twin
is `src/clipboard.rs:177`.

### `clipboard::copy_to_clipboard_auto(text)` — write + scheduled clear

```rust
// src/clipboard.rs:166
pub fn copy_to_clipboard_auto(text: &str) -> SnipResult<()> {
    let auto_clear_seconds = get_clipboard_auto_clear_seconds();
    copy_to_clipboard_with_auto_clear(text, auto_clear_seconds)
}
```

`copy_to_clipboard_with_auto_clear` (`src/clipboard.rs:155`) performs the copy
first and only then schedules the clear, so a failed copy never arms a timer
(`:159-162`). The delay comes from `clipboard_auto_clear_seconds` in the sync
settings (`src/config/sync_settings.rs:124`, `Option<u32>`, `None` by default
at `:407`), so **auto-clear is off unless the user configures it**.

Still no audit entry and no usage-index update — this is the raw OS layer.

### `clip_cmd::copy_to_clipboard(snippet, final_command)` — the funnel

```rust
// src/commands/clip_cmd.rs:40
pub(crate) fn copy_to_clipboard(snippet: &Snippet, final_command: &str) -> SnipResult<()> {
    crate::clipboard::copy_to_clipboard_auto(final_command)?;
    crate::logging::audit_log("copy", snippet, None)?;
    let mut usage_idx = crate::usage::UsageIndex::load();
    usage_idx.record_use(&snippet.id);
    if let Err(e) = usage_idx.save() {
        tracing::debug!("Usage save failed: {}", e);
    }
    Ok(())
}
```

Three effects, in this order: the OS write, the audit row, the usage-index
bump. Variable expansion happens **before** the call — `process_snippet`
(`src/commands/clip_cmd.rs:51-61`), `run_exact` (`:83-88`), and
`snp run --copy` (`src/commands/run_cmd.rs:202`) all expand first and pass
the final string in. The audit failure propagates (`?` at `:42`); the usage
save is best-effort and only logged at debug level (`:45-47`).

Callers of the funnel:

| Caller | Location | Notes |
|--------|----------|-------|
| `clip_cmd::run` | `src/commands/clip_cmd.rs:99` | TUI selection → `process_snippet` (`:61`) |
| `clip_cmd::run_exact` | `src/commands/clip_cmd.rs:68` | `--id` / `--description-exact` / `--command-exact`; copy at `:88`, then optional post-clip explicit sync (`:89-94`) |
| `run_cmd::process_snippet` | `src/commands/run_cmd.rs:201-202` | `snp run --copy` |

## Timeout guard

`with_clipboard_timeout(op, f)` (`src/clipboard.rs:112`) runs the OS call on a
worker thread and waits on `rx.recv_timeout(timeout)` (`:128`). The timeout
comes from `SNP_CLIPBOARD_TIMEOUT` (seconds, parsed as `u64`), defaulting to
`DEFAULT_CLIPBOARD_TIMEOUT_SECS = 5` (`src/clipboard.rs:26`, `:62-68`).

- **Timeout** (`:130-142`): the worker handle is dropped, so the OS call
  continues in the background — fire-and-forget by design (`:119-123`); at
  most one outstanding thread per user-invoked operation, so accumulation is
  bounded by the user action rate, not by a retry loop. A `tracing::warn` is
  emitted (`:131-135`) and `SnipError::clipboard_error(op, "clipboard
  operation timed out after N seconds")` is returned (`:136-142`).
- **Channel disconnect** (`:144-147`): a separate error, `"clipboard
  operation channel closed unexpectedly"`.

`clear_clipboard()` (`src/clipboard.rs:151`) uses the same guard with
operation name `"clear"`.

## Auto-clear

```rust
// src/clipboard.rs:75
pub fn schedule_clipboard_clear(seconds: u32) {
    if seconds == 0 {
        return;
    }

    let gen_at_spawn = CLIPBOARD_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    let _handle = thread::spawn(move || {
        thread::sleep(Duration::from_secs(seconds as u64));
        if gen_at_spawn == CLIPBOARD_GENERATION.load(Ordering::SeqCst)
            && let Err(e) = clear_clipboard()
        {
            tracing::warn!("Auto-clear clipboard failed: {}", e);
        }
    });
}
```

- **Zero is a no-op** (`:76-78`).
- **Generation counter** (`src/clipboard.rs:24`): every scheduled clear bumps
  `CLIPBOARD_GENERATION` (`:80`); the thread only clears if its own
  generation is still current (`:84`). A newer copy therefore cancels a
  pending clear — the stale clear never wipes fresh content.
- **Detached thread** (`:82`): the handle is dropped immediately, so the
  thread is not joined. Its module doc (`:70-74`) states the consequence
  explicitly: *if the process exits first, the clear is skipped* — it is a
  best-effort security measure, not a guarantee.
- **Failures warn, they do not fail the copy** (`:85-88`): the error is
  logged at **warn** level (`tracing::warn!("Auto-clear clipboard failed:
  {}")`, `:87`), not debug.

Because the clear runs on a detached thread inside the same process, it is
only observable for a long-lived process — i.e. the TUI (`snp` with no
subcommand). A short-lived `snp clip` exits before the timer fires.

## Audit interaction

`log_clipboard_operation(operation, success)` (`src/logging.rs:356-362`)
records **debug** on success and **warn** on failure. It logs the operation
name and a boolean only — never the copied text.

Every OS **copy** inside `src/clipboard.rs` calls it on success and failure;
`clear_clipboard_impl` calls it only on success:

| Site | Operation string | Level |
|------|------------------|-------|
| Windows copy success | `"set_text"` | debug (`src/clipboard.rs:185`) |
| Windows copy failure | `"set_text"` | warn (`src/clipboard.rs:181`) |
| arboard context creation failure | `"create context"` | warn (`src/clipboard.rs:200`) |
| arboard copy success | `"set_text"` | debug (`src/clipboard.rs:209`) |
| arboard copy failure | `"set_text"` | warn (`src/clipboard.rs:205`) |
| Clear success | `"clear_clipboard"` | debug (`src/clipboard.rs:108`) |

Note that `clear_clipboard_impl` logs success only on the happy path — a
failed clear returns `Err` before reaching `:108`, and the auto-clear thread
logs the failure itself at warn level (`src/clipboard.rs:87`).

## Direct call sites

Exactly two sites call `clipboard::copy_to_clipboard_auto` directly,
bypassing `clip_cmd::copy_to_clipboard`. Their audit behaviour differs, so
they are **not** symmetric.

### TUI visual-range multi-select

`src/ui/mod.rs:1733`:

```rust
// src/ui/mod.rs:1726-1733
let selected_items: Vec<String> = filtered
    .iter()
    .skip(start)
    .take(end - start + 1)
    .map(|idx| strip_escape_sequences(&commands[*idx]))
    .collect();
let copy_text = selected_items.join("\n");
match clipboard::copy_to_clipboard_auto(&copy_text) {
```

- The copied payload is the **commands** of the range (the raw
  `snippet.command` strings, escape-stripped by
  `strip_escape_sequences`, `src/utils/variables.rs:61`), joined with `\n`
  (`:1641`) — **not** the expanded snippet text and **not** the descriptions.
- **Audit covers only the first row of the range**: `filtered.get(start)`
  (`:1657`) is the only snippet passed to `crate::logging::audit_log("copy",
  ...)` (`:1660-1664`). A 10-row copy produces one audit row.
- **No usage-index update** — `UsageIndex::record_use` is only reached
  through `clip_cmd::copy_to_clipboard`.
- A copy failure is surfaced in the TUI status line and logged at warn level
  (`:1650-1654`); the audit row is still written afterwards (`:1657-1668`),
  so a failed copy can still produce an audit entry.
- The TUI then returns `SnippetSelection::Copied` (`:1671`), which
  `run_snippet_selection` treats as "selected and processed"
  (`src/commands/mod.rs:556-559`) — so `clip_cmd::process_snippet` never
  runs for this path either.

### `snp cron` prompt

`src/commands/cron_cmd.rs:89`:

```rust
// src/commands/cron_cmd.rs:88-92
if io::stdin().read_line(&mut input).is_ok() && input.trim().to_lowercase() == "y" {
    match crate::clipboard::copy_to_clipboard_auto(&cron_entry) {
        Ok(()) => println!("Copied to clipboard!"),
        Err(e) => eprintln!("Failed to copy to clipboard: {e}"),
    }
}
```

- Copies the generated crontab entry.
- **No audit log at all** and no usage-index update — there is no snippet
  involved.
- A failure is printed to stderr (`:91`); no `tracing` event is emitted.

## Error model

Every failure is `SnipError::clipboard_error(operation, detail)`
(`src/error.rs:280-285`), producing the variant
`SnipError::Clipboard { operation: String, message: String }`
(`src/error.rs:130`), rendered as `Clipboard error during {operation}:
{message}` (`src/error.rs:178-180`).

| Case | `operation` | `message` | Location |
|------|-------------|-----------|----------|
| arboard context creation fails | `"create clipboard context"` | underlying arboard error | `src/clipboard.rs:201` |
| arboard `set_text` fails | `"set text"` | underlying arboard error | `src/clipboard.rs:206` |
| Windows `set_clipboard` fails | `"set text"` | underlying `clipboard-win` error | `src/clipboard.rs:182` |
| Clear fails (arboard) | `"clear clipboard"` | underlying arboard error | `src/clipboard.rs:105` |
| Clear context creation fails (arboard) | `"create clipboard context for clear"` | underlying arboard error | `src/clipboard.rs:102` |
| Clear fails (Windows) | `"clear clipboard"` | underlying `clipboard-win` error | `src/clipboard.rs:96` |
| Exceeds `SNP_CLIPBOARD_TIMEOUT` | `"copy"` or `"clear"` | `"clipboard operation timed out after N seconds"` | `src/clipboard.rs:136-142` |
| Worker channel drops | `"copy"` or `"clear"` | `"clipboard operation channel closed unexpectedly"` | `src/clipboard.rs:144-147` |

Timeouts additionally emit a `tracing::warn` naming the operation and the
timeout in seconds (`src/clipboard.rs:131-135`).

## Settings cache

`CACHED_CLIPBOARD_SETTINGS` (`src/clipboard.rs:32-33`) memoizes
`clipboard_auto_clear_seconds` so every copy avoids a settings read.
`get_clipboard_auto_clear_seconds` (`:35-53`) fills it from
`crate::config::get_sync_settings()` (`:44`); `invalidate_clipboard_settings_cache`
(`:55-60`) clears it. The only invalidator is
`save_sync_settings()` (`src/config/sync_settings.rs:501`) — so a settings
change made by another process is not observed until the next write.

Poisoned-mutex guards (`unwrap_or_else(|e| e.into_inner())`, `:39`, `:48`,
`:57`) keep a panicked holder from wedging future copies.

## Platform edge cases

### Headless / no display server

`arboard::Clipboard::new()` requires an X server on Linux. With `DISPLAY`
unset or the server unreachable, arboard returns
`"X11 server connection timed out because it was unreachable"`, which
snip-it maps to `SnipError::clipboard_error("create clipboard context", ...)`
(`src/clipboard.rs:201`). `snp clip` therefore fails on a headless server
rather than silently succeeding — there is no in-memory fallback clipboard.

### Wayland

This build has **no Wayland support** (see
[Dependency gating](#dependency-gating-cargotoml)). Under a pure Wayland
session with no XWayland and no `DISPLAY`, `Clipboard::new()` fails and every
copy errors out. `WAYLAND_DISPLAY` is never read by snip-it or by the
compiled arboard backend. Enabling arboard's `wayland-data-control` feature
would be required for native Wayland support; that is a `Cargo.toml` change
and out of scope here.

### X11 selection lifetime

The X11 clipboard is a **selection owned by a process**, not a buffer in the
X server. arboard keeps a process-global X11 server thread that serves the
selection after `set_text` returns, and `set_text` uses
`WaitConfig::None` — it does not wait for the content to be consumed. When
the owning process exits, the selection is gone unless a clipboard manager
claimed it (arboard asks the manager to take the data when the last owner is
dropped, which does not happen on an abrupt exit). For snip-it this means:

- `snp clip` copies text that may vanish when the CLI process exits.
- The scheduled auto-clear cannot run at all in that case — the detached
  thread dies with the process.

### Windows clipboard contention

`clipboard_win::set_clipboard` opens the clipboard with 10 attempts
(`clipboard-win` `lib.rs:234`, `new_attempts` at `lib.rs:148-161`), yielding
between attempts. If another process holds the clipboard open, the call can
still fail with an `OpenClipboard` error, which snip-it maps to
`SnipError::clipboard_error("set text", ...)` (`src/clipboard.rs:182`).
Unlike X11, the Windows clipboard content lives in the session and **survives
process exit** — but the auto-clear thread still dies with the process, so
`clipboard_auto_clear_seconds` has no effect for short-lived CLI commands.

## Tests

Unit tests in `src/clipboard.rs:214-260` cover empty, normal, unicode,
multiline, special-char, and 100 K-char payloads. **All six are `#[ignore]`**
— each needs a display server, so they do not run on headless CI.

| Test | Location | Payload |
|------|----------|---------|
| empty string | `src/clipboard.rs:220` | `""` |
| normal text | `src/clipboard.rs:227` | `"test content"` |
| unicode | `src/clipboard.rs:234` | `"héllo wörld 🎉"` |
| multiline | `src/clipboard.rs:241` | three `\n`-joined lines |
| special chars | `src/clipboard.rs:248` | pipes and quotes |
| 100 K-char content | `src/clipboard.rs:255` | `"x".repeat(100000)` |

`src/commands/clip_cmd.rs` has the same split: the two live-clipboard tests
are `#[ignore]` (`:124`, `:137`) while the "sync requires a runtime" test
(`:150`) runs normally — it fails before touching the clipboard.

The live clipboard behaviour is covered by an integration test instead:
`tests/pty_integration.rs:1554-1563` skips
`test_pty_clip_sort_last_used_records_usage_for_c` on Linux when neither
`DISPLAY` nor `WAYLAND_DISPLAY` is set, and otherwise asserts that `snp clip`
bumps the usage index — i.e. that the copy went through
`clip_cmd::copy_to_clipboard`, not the raw OS layer.

## Key Files

| File | Subject |
|------|---------|
| `src/clipboard.rs` (260 lines) | Platform backends, timeout guard, auto-clear, settings cache |
| `src/commands/clip_cmd.rs:40` | `copy_to_clipboard()` — the sanctioned side-effect funnel |
| `src/ui/mod.rs:1726-1763` | TUI visual-range multi-select — direct `copy_to_clipboard_auto` call |
| `src/commands/cron_cmd.rs:89` | `snp cron` prompt — direct `copy_to_clipboard_auto` call, no audit |
| `src/commands/run_cmd.rs:194-207` | `snp run --copy` — goes through the funnel |
| `src/error.rs:130`, `:280` | `SnipError::Clipboard` variant and constructor |
| `src/logging.rs:356` | `log_clipboard_operation` — op name + success flag only |
| `src/config/sync_settings.rs:124`, `:493` | `clipboard_auto_clear_seconds` and its cache invalidator |
| `src/utils/variables.rs:61` | `strip_escape_sequences` — applied to TUI batch copies |
| `Cargo.toml:98-103` | Target-gated `clipboard-win` / `arboard` dependencies |

## Invariants

1. **One funnel for snippet copies.** `clip_cmd::copy_to_clipboard`
   (`src/commands/clip_cmd.rs:40`) is the sanctioned path: it is the only
   one that writes the audit row and bumps the usage index.
2. **Exactly two bypasses, with deliberately different audit behaviour.**
   The TUI visual-range copy audits only the first row
   (`src/ui/mod.rs:1748`); the `snp cron` prompt audits nothing
   (`src/commands/cron_cmd.rs:89`).
3. **Clipboard content is never logged.** Only operation names and success
   flags reach `tracing` (`src/logging.rs:356-362`).
4. **The generation counter prevents a stale auto-clear from wiping a newer
   copy** (`src/clipboard.rs:24`, `:80`, `:84`).
5. **The auto-clear is best-effort.** It runs on a detached thread and is
   skipped when the process exits first (`src/clipboard.rs:70-74`).
6. **Timeouts bound hung OS calls.** The worker is detached on timeout, and
   outstanding workers are bounded by the user action rate
   (`src/clipboard.rs:119-127`).
7. **Every clipboard failure is a typed `SnipError::Clipboard`** — never a
   silent no-op and never a panic.

## Gotchas

- **Three functions, two of them nearly identical.** `clipboard::copy_to_clipboard`
  and `clip_cmd::copy_to_clipboard` differ by one underscore position and by
  two side effects. Use the `clip_cmd::` one for snippet copies.
- **`copy_to_clipboard_auto` is not "automatic auditing".** It only adds the
  *auto-clear*; it adds no audit row and no usage-index update.
- **The TUI visual-range copy pastes commands, not descriptions.** The
  payload is `strip_escape_sequences(command)` joined with `\n`
  (`src/ui/mod.rs:1730-1732`). A reader expecting descriptions will get
  shell text.
- **A multi-row TUI copy produces exactly one audit row.** `filtered.get(start)`
  (`src/ui/mod.rs:1748`) is the only audited snippet; the remaining rows are
  invisible to the audit trail.
- **A failed TUI copy can still be audited.** The audit write at
  `src/ui/mod.rs:1751` happens after the copy result is handled, so the
  status line can say "Copy failed" while `audit.log` records a `copy`.
- **The auto-clear never fires for one-shot CLI commands.** `snp clip` exits
  before the timer; only the long-running TUI benefits.
- **Auto-clear is off by default.** `clipboard_auto_clear_seconds` is
  `None` unless the user sets it (`src/config/sync_settings.rs:407`).
- **This build has no Wayland support.** `arboard` is pulled with
  `default-features = false` (`Cargo.toml:102`), which disables
  `wayland-data-control`; the Linux backend is X11-only.
- **The settings cache is invalidated only by `save_sync_settings`.** An
  external edit to the config file is not picked up until the next in-process
  settings write (`src/config/sync_settings.rs:501`).
- **`SNP_CLIPBOARD_TIMEOUT` is parsed as `u64` seconds.** A malformed or
  negative value silently falls back to 5 seconds
  (`src/clipboard.rs:62-68`).
