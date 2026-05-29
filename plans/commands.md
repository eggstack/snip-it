# Commands Architecture Review - Improvement Plan

## Architecture Document Verification

### Document Claims vs Actual Implementation

| Claim | Status | Notes |
|-------|--------|-------|
| **mod.md: get_config_path() returns PathBuf for ~/.config/snp/** | ✓ Verified | Line 51: `Ok(crate::CONFIG_PATH.clone())` |
| **mod.md: get_library_path() returns path to snippets.toml or active library file** | ✓ Verified | Lines 56-84 |
| **mod.md: load_snippets() reads TOML, returns empty if missing, handles migration** | ✓ Verified | Lines 96-135 |
| **mod.md: save_snippets() writes TOML, creates dirs, no automatic backup** | ✓ Verified | Lines 138-158 |
| **mod.md: get_snippet_data() returns SnippetData** | ✓ Verified | Lines 163-188, returns SnippetData struct |
| **mod.md: expand_snippet_command() syntax <name> or <name=default>** | ✓ Verified | Lines 190-208 |
| **mod.md: run_snippet_selection() common flow for run/clip/search** | ✓ Verified | Lines 210-265 |
| **clip_cmd.md: Entry point run(matches: &ArgMatches)** | **INACCURATE** | Actual: `run(filter, do_sync, library, config, runtime)` |
| **clip_cmd.md: --clear <seconds> flag for auto-clear** | **NOT IMPLEMENTED** | No such flag in CLI; clipboard.rs has `schedule_clipboard_clear` but unused |
| **clip_cmd.md: Platform-specific clipboard via clipboard-win/copypasta** | ✓ Verified | clipboard.rs:14-18 |
| **clip_cmd.md: Generation tracking to avoid self-clear** | ✓ Verified | clipboard.rs:23, 30-35 |
| **cron_cmd.md: Load sync settings from sync.toml** | **INACCURATE** | Does not load sync settings; just takes interval directly |
| **cron_cmd.md: Generated entry uses --local flag** | **INACCURATE** | Actual: uses `--non-interactive` (line 17) |
| **cron_cmd.md: Interval mapping table (15→*/15, 60→0 *, etc.)** | **NOT IMPLEMENTED** | Actual: just does `*/{interval}` for all values |
| **cron_cmd.md: --install flag appends crontab, --remove removes entries** | **NOT IMPLEMENTED** | Actual: only prints instructions |
| **cron_cmd.md: Uses crontab - for safe read/write** | **NOT IMPLEMENTED** | No crontab manipulation at all |
| **edit_cmd.md: $EDITOR env var → platform default fallback** | **INACCURATE** | Actual: defaults to "vim" not platform default |
| **edit_cmd.md: Command::new(editor).arg(path).spawn()** | **INACCURATE** | Actual: uses `.status()` not `.spawn()`, waits for exit |
| **edit_cmd.md: Clap value parser for known editors** | **NOT IMPLEMENTED** | Actual: PATH resolution, no known-editor checking |
| **edit_cmd.md: Error variants SnipError::Command/Io/Toml** | ✓ Verified | Lines 33-35 |
| **keybindings_cmd.md: TUI-based help screen** | **INACCURATE** | Actual: plain stdout print, no TUI |
| **keybindings_cmd.md: Navigation keybindings table** | **INACCURATE** | Actual keybindings differ significantly from documented |
| **keybindings_cmd.md: q/Esc to quit** | **INACCURATE** | Just prints and exits, no quit detection |
| **keybindings_cmd.md: Future support for keybindings.toml** | ✓ Verified | "Future versions" - currently not implemented |
| **library_cmd.md: list/create/delete/set-primary/show subcommands** | ✓ Verified | All present in main.rs:183-199 |
| **library_cmd.md: Migration from snippets.toml to libraries/** | ✓ Verified | library.rs:241-265 |
| **library_cmd.md: libraries.toml metadata format** | ✓ Verified | library.rs:67-85 |
| **list_cmd.md: --json and --csv output formats** | **NOT IMPLEMENTED** | Actual: only plain text output |
| **list_cmd.md: --tag, --folder, --sort filters** | **NOT IMPLEMENTED** | Actual: only `--filter` option |
| **list_cmd.md: Fuzzy search on name/command** | ✓ Verified | Lines 33-35 |
| **new_cmd.md: Name → command → optional fields → save flow** | ✓ Verified | Lines 27-93 |
| **new_cmd.md: Multiline input (Enter=new line, Ctrl+D=finish, Ctrl+C=cancel)** | **INACCURATE** | Actual: double-empty-line terminator, not Ctrl+D |
| **new_cmd.md: Tags via comma-separated input** | ✓ Verified | Lines 63-74 |
| **new_cmd.md: Folder organization** | **NOT IMPLEMENTED** | No folder input in new_cmd |
| **new_cmd.md: Favorite boolean flag** | **NOT IMPLEMENTED** | No favorite input in new_cmd |
| **new_cmd.md: Backup before save via backup_library()** | **NOT IMPLEMENTED** | backup_library never called in new_cmd |
| **new_cmd.md: Sort by updated_at descending after save** | **NOT IMPLEMENTED** | No sorting after push |
| **premade_cmd.md: list/get/sync subcommands** | ✓ Verified | main.rs:203-211 |
| **premade_cmd.md: Downloads to ~/.config/snp/premade/** | ✓ Verified | library.rs:433-454 |
| **premade_cmd.md: Merge snippets into local library** | **NOT IMPLEMENTED** | Just downloads, no merge |
| **register_cmd.md: URL input, credentials, registration, keychain, config** | ✓ Verified | Lines 6-73 |
| **register_cmd.md: --server, --name, --api-key flags** | **INACCURATE** | Only --server flag exists |
| **register_cmd.md: Error variants SnipError::Sync/Keychain/InvalidCredentials** | **PARTIALLY** | Only Sync/Keychain errors exist; InvalidCredentials not used |
| **run_cmd.md: TUI selection → variable expansion → execute → output → clipboard → audit** | ✓ Verified | Lines 74-135 |
| **run_cmd.md: Shell from $SHELL, fallback to /bin/sh or cmd.exe** | ✓ Verified | Lines 48-50 |
| **run_cmd.md: --clip flag copies output to clipboard** | **INACCURATE** | --clip copies the command, not the output |
| **run_cmd.md: --sync flag syncs after execution** | ✓ Verified | Line 144 |
| **run_cmd.md: Audit log records name, timestamp, expanded command, exit code** | ✓ Verified | Lines 52-72 |
| **search_cmd.md: Fuzzy matching via fuzzy-matcher SkimMatcherV2** | ✓ Verified | Uses run_snippet_selection which uses SkimMatcherV2 |
| **search_cmd.md: z key toggles display modes (Normal/Detailed)** | **NOT IMPLEMENTED** | Just prints details, no mode toggle |
| **sync_cmd.md: --local for local-only sync** | **NOT IMPLEMENTED** | Actual uses --non-interactive; no local-only mode |
| **sync_cmd.md: --servers lists server libraries** | ✓ Verified | Lines 141-162 |
| **sync_cmd.md: Bidirectional sync with last-write-wins merge** | ✓ Verified | sync_commands.rs:394-475 |
| **sync_cmd.md: --interval flag for periodic sync** | **NOT IMPLEMENTED** | No such flag in CLI |
| **sync_cmd.md: Settings from ~/.config/snp/sync.toml** | ✓ Verified | Lines 133-139 |

---

## Bugs & Edge Cases

### 1. **cron_cmd Generates Invalid Cron for Intervals >= 60**

**Location**: `src/commands/cron_cmd.rs:16-18`

**Issue**: For intervals >= 60, the cron entry `*/60 * * * *` means "every 60 minutes" but syntax `*/N` where N > 59 is invalid in most cron implementations.

```rust
let cron_entry = format!(
    "*/{} * * * * {} sync --non-interactive",
    interval, binary_path
);
```

**Impact**: Users who set `--interval 60` get an invalid crontab entry.

---

### 2. **clip_cmd Never Uses Auto-clear Despite Documentation**

**Location**: `src/commands/clip_cmd.rs:1-41`

**Issue**: The documentation claims `--clear <seconds>` flag exists and auto-clears clipboard. Neither exists in the CLI or the command. The `clipboard::schedule_clipboard_clear` function exists but is never called.

**Code**: `clip_cmd` uses `copy_to_clipboard_auto` which only auto-clears based on sync settings, not per-command flags.

---

### 3. **run_cmd --clip Copies Command, Not Output**

**Location**: `src/commands/run_cmd.rs:81-90`

**Issue**: Documentation says `--clip` "Copy output to clipboard after execution" but actual code copies the expanded command:

```rust
if copy {
    crate::clipboard::copy_to_clipboard_auto(&final_command)?;  // This is the COMMAND
```

**Impact**: User expectation mismatch - if a snippet produces output (like `echo "hello"`), --clip copies `echo "hello"` not `hello`.

---

### 4. **new_cmd Multiline Terminates on Double Empty Line**

**Location**: `src/commands/new_cmd.rs:8-25`

**Issue**: Multiline input reads until TWO consecutive empty lines. This means:
- A snippet command cannot contain an empty line as part of its content
- The terminator is indistinguishable from intentional empty lines in the command

```rust
if line.trim().is_empty() && prev_was_empty {
    break;
}
```

**Impact**: Cannot create snippets with internal empty lines (e.g., multi-paragraph output scripts).

---

### 5. **list_cmd --filter is Fuzzy But Documentation Claims Multiple Filter Types**

**Location**: `src/commands/list_cmd.rs:28-40`

**Issue**: Documentation claims `--tag`, `--folder`, `--search`, and `--sort` filters. Actual implementation only has `--filter` which does fuzzy matching on "description + command". No tag filtering, no folder filtering, no sorting.

```rust
let filtered: Vec<_> = if let Some(ref filter_str) = filter {
    snippets.snippets.iter().enumerate().filter(|(_, s)| {
        let display = format!("{} {}", s.description, s.command);
        matcher.fuzzy_match(&display, filter_str).is_some()
    }).collect()
```

**Impact**: Users expecting CLI like `snp list --tag git` will be disappointed.

---

### 6. **sync_cmd Doesn't Support --local Despite Documentation**

**Location**: `src/commands/sync_cmd.rs:124-199`, `src/main.rs:142-153`

**Issue**: Documentation says `--local` flag performs "local-only sync without server" but this flag doesn't exist. The CLI has `--non-interactive` but that's different.

**Code**: Looking at `sync_cmd::run`, it always creates a sync client and connects to server unless `--servers` is set.

**Impact**: Users cannot perform local-only operations as documented.

---

### 7. **register_cmd Silently Overwrites Existing Registration**

**Location**: `src/commands/register_cmd.rs:6-17`

**Issue**: If already registered, it prints a message and returns `Ok(())`. This means `snp register` on an already-registered device does nothing - no re-registration, no error, just silent success.

```rust
if let Ok(settings) = load_sync_settings() {
    if !settings.device_id.is_empty() {
        eprintln!("Already registered! ...");
        return Ok(());  // Silent early exit
    }
}
```

**Impact**: Cannot re-register without manually editing config file.

---

### 8. **premade_cmd Does Not Merge, Only Downloads**

**Location**: `src/commands/premade_cmd.rs:43-141`

**Issue**: Documentation says "Merge snippets into local library (optional)" but `run_get` only saves to the premade directory, never merges into primary library.

```rust
let path = mgr.save_premade_library(&name, &content)?;  // Just saves, no merge
```

**Impact**: Premade libraries are stored separately and never integrated.

---

### 9. **search_cmd Ignores Copy Flag from TUI**

**Location**: `src/commands/search_cmd.rs:12-20`

**Issue**: `run_snippet_selection` passes `copy_flag` to the closure, but `search_cmd` ignores it (`_copy_flag`). If user presses 'y' in TUI to copy, search still just prints details.

```rust
run_snippet_selection(filter, library, do_sync, runtime, |snippet, _copy_flag| {
    println!("Description: {}", snippet.description);
    // copy_flag is ignored!
```

---

## Security Concerns

### 1. **Output Path Validation Uses String Comparison, Not Canonicalization**

**Location**: `src/commands/run_cmd.rs:10-46`

**Issue**: `validate_output_path` checks for `..` components and absolute paths via string/representation checks, not actual path resolution. A symlink attack could bypass these checks.

```rust
for component in p.components() {
    match component {
        std::path::Component::ParentDir => { return Err(...) }
        std::path::Component::Normal(c) if c.to_string_lossy().contains("..") => { return Err(...) }
```

**Recommendation**: Use `std::fs::canonicalize` to resolve the actual path and verify it stays within the working directory.

---

### 2. **Editor Path Resolution Respects Directory Components**

**Location**: `src/commands/edit_cmd.rs:39-98`

**Issue**: Relative paths with directory components (e.g., `./script.sh`) are resolved against CWD. An attacker could place a malicious `./vim` in a directory and wait for user to edit from there.

```rust
if has_directory_component(editor) {
    let candidate = cwd.join(editor);  // Resolves relative to CWD
```

**Impact**: If user has CWD pointing to an attacker-controlled directory and EDITOR is set to a bare name, they get the attacker's binary.

---

### 3. **API Key Masked but Still Printed to stdout**

**Location**: `src/commands/register_cmd.rs:57-62`

**Issue**: API key is masked before printing (`{}...{}`) but the full key is constructed in memory. Someone with access to process memory or core dumps could recover it.

```rust
let masked_key = if api_key.len() > 8 {
    format!("{}...{}", &api_key[..4], &api_key[api_key.len() - 4..])
} else {
    "****".to_string()
};
println!("API key: {}", masked_key);
```

---

## Missing Error Handling

### 1. **sync_cmd::run Doesn't Propagate Sync Errors**

**Location**: `src/commands/sync_cmd.rs:185-192`

```rust
crate::sync_commands::run_sync(...);
// No error checking - sync failures are silently ignored
Ok(())  // Always returns Ok
```

**Impact**: User has no indication if sync succeeded or failed.

---

### 2. **premade_cmd::run_sync Ignores Return Value**

**Location**: `src/commands/premade_cmd.rs:144-153`

```rust
pub fn run_sync(runtime: &tokio::runtime::Runtime) -> SnipResult<()> {
    let sync_settings = get_sync_settings();
    if !sync_settings.enabled { ... }
    crate::sync_commands::run_premade_sync(&sync_settings, runtime);
    Ok(())  // Always Ok
}
```

---

### 3. **run_cmd::process_snippet Returns Different Types**

**Location**: `src/commands/run_cmd.rs:74-135`

The function returns `Ok(crate::ProcessResult::Done("Copied to clipboard"))` when copy succeeds but `Ok(handle_command_result(...))` when executing. Error handling paths are inconsistent - some errors are logged but return Ok.

---

## Performance Considerations

### 1. **Multiple LibraryManager Instantiations Per Command**

**Location**: Throughout commands

`get_library_path()` creates a `LibraryManager`, then `run_snippet_selection()` may call `crate::library::load_library(&lib_path)` creating another one. Each instantiation reads from disk.

**Affected commands**: `run_cmd`, `clip_cmd`, `search_cmd`, `list_cmd`, `edit_cmd`, `new_cmd`

---

### 2. **run_snippet_selection Reloads Library on Each Iteration**

**Location**: `src/commands/mod.rs:230`

```rust
let snippets = crate::library::load_library(&lib_path)?;  // Loaded once
// ...
loop {
    let result = crate::ui::select_snippet(...)?;  // TUI selection
    // If Continue, loop restarts but library is not reloaded
}
```

Not a bug currently, but if snippet file changes externally during TUI session, changes won't be reflected.

---

## Discrepancies Summary

| Module | Critical Discrepancies |
|--------|----------------------|
| **clip_cmd** | No --clear flag; auto-clear undocumented |
| **cron_cmd** | No --install/--remove; --local not supported; interval mapping fake |
| **edit_cmd** | No known-editor validation; vim hardcoded; .status() not .spawn() |
| **keybindings_cmd** | Not TUI; keybindings don't match docs |
| **list_cmd** | No --json/--csv/--tag/--folder/--sort |
| **new_cmd** | No folders/favorites; double-empty-line terminator; no backup |
| **premade_cmd** | No merge into local library |
| **register_cmd** | No --name/--api-key flags |
| **run_cmd** | --clip copies command not output |
| **search_cmd** | No display mode toggle; ignores copy flag |
| **sync_cmd** | No --local; no --interval |

---

## Potential Improvements

### High Priority

1. **Implement missing CLI flags** to match documentation:
   - `--clear` for clip_cmd
   - `--tag`/`--folder`/`--sort` for list_cmd
   - `--local` for sync_cmd
   - `--interval` for sync_cmd

2. **Fix cron_cmd interval handling**:
   - For `--interval 60` or higher, use `0 */1 * * *` format instead of `*/60`
   - Implement actual `--install`/`--remove` functionality

3. **Add actual merge for premade libraries** or update documentation to clarify they are view-only

4. **Fix run_cmd --clip behavior** to copy output (if that's intended) or update docs to say "copies command"

### Medium Priority

5. **Add error propagation in sync_cmd::run** and premade_cmd::run_sync

6. **Use canonicalize for output path validation** in run_cmd

7. **Replace double-empty-line terminator** with Ctrl+D or another mechanism in new_cmd

8. **Make keybindings_cmd actually TUI-based** or update docs to say it's a print command

9. **Add LibraryManager caching** within command execution context

### Low Priority

10. **Add re-registration support** or force flag for register_cmd
11. **Mask/unmask API key handling** to avoid memory exposure
12. **Add --dry-run for sync**
13. **Add timeout for editor** in edit_cmd
14. **Consider --json/--csv for list_cmd** for scripting support

---

## Verified Correct Behavior

- Signal handling differs correctly between Unix and Windows (main.rs:47-61)
- Clipboard uses platform-appropriate backend (copypasta/clipboard-win)
- TOML escaping/unescaping for backslashes works (toml_helpers.rs)
- Fuzzy matching combines description + command correctly
- Library mode transitions with automatic migration work
- Audit logging failures are non-fatal
- Editor resolution searches PATH correctly
- Sync merge preserves local-only fields (output, folders, favorite)
- Deleted snippets handled correctly in merge (marked deleted, not removed)
