# Pet Compatibility Matrix

This document defines the behavioral contract between snip-it (`snp`) and [pet](https://github.com/knqyf263/pet) for the pet-migration compatibility work. It covers what works today, what will change, and what stays the same.

**Compatibility does not mean cloning pet's architecture or defaults.** snp is an opinionated terminal-first snippet manager with its own native TUI, named libraries, richer local metadata, encrypted self-hosted synchronization, and integrated themes. The compatibility objective is narrower: existing pet snippet data should import predictably, and the shell-buffer workflow that makes pet feel like an enhanced shell history should be available as an opt-in snp integration.

## Table Format

For each feature area:

- **Feature**: What the feature is
- **Pet behavior**: How pet handles it
- **snp current behavior**: How snp handles it today
- **Compatibility status**: Full / Partial / None / New
- **Release target**: Which release addresses gaps (R1A-R5)
- **Notes**: Details

---

## 1. Snippet File Format

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Table array name | `[[snippets]]` (lowercase) | Reads `[[snippets]]` (lowercase, pet canonical). Also reads `[[Snippets]]` via alias. | Full | — | snp serializes to lowercase `[[snippets]]` on save, matching pet's canonical form. |
| `description` field | `Description` (PascalCase) or `description` (lowercase) | Reads both `Description`, `name`, and `description` via serde aliases. Serializes lowercase `description`. | Full | — | pet also supports `name` as alias for `description`; snp reads this. |
| `command` field | `Command` (PascalCase) or `command` (lowercase) | Reads both `Command`, `cmd`, and `command` via serde aliases. Serializes lowercase `command`. | Full | — | pet also supports `cmd` as alias; snp reads this. |
| `tag` field | `Tag` (PascalCase), `Tags`, `tags`, `tag` (lowercase) | Reads all four variants via serde aliases. Serializes `tag = [...]`. | Full | — | Pet uses PascalCase `Tag`; snp accepts both. |
| `output` field | `Output` (PascalCase) or `output` (lowercase) | Reads both via aliases. Serializes `output = ""`. | Full | — | Pet displays output as metadata; snp preserves it but does not display in TUI by default. |
| `id` field | Not present in pet | `id` (UUID, auto-generated on load if empty). Reads `Id`, `ID` aliases. | New | — | snp-only. Pet files get IDs auto-assigned on first load. |
| `created_at` / `updated_at` | Not present in pet | Auto-populated on creation. Preserved through round-trips. | New | — | snp-only timestamps (unix epoch seconds). |
| `device_id` | Not present in pet | Used for sync conflict resolution. | New | — | snp-only. Empty string in non-synced snippets. |
| `deleted` | Not present in pet | Tombstone flag for sync. Deleted snippets are filtered from TUI display but preserved for sync propagation. | New | — | snp-only. Pet has no concept of soft-delete. |
| `folders` | Not present in pet | Array of folder names for organizational grouping. | New | — | snp-only. |
| `favorite` | Not present in pet | Boolean flag for favorites. | New | — | snp-only. |
| TOML backslash handling | Standard TOML escaping | `fix_invalid_toml_escapes()` converts double-quoted strings with problematic backslashes to single-quoted raw literals on read. `quote_strings_containing_backslashes()` reverses on save. | Supported differently | — | snp handles pet files with `\<` and `\>` in double-quoted strings more permissively than strict TOML. |

### Serialization contract

snp always writes:
- Lowercase `[[snippets]]` table name
- Lowercase field names: `description`, `command`, `tag`, `output`
- snp-only fields: `id`, `created_at`, `updated_at`, `device_id`, `deleted`, `folders`, `favorite`

This means snp output is loadable by pet (pet ignores unknown fields), but snp output may contain fields pet does not display.

---

## 2. Multi-Library Support

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Default config path | `~/.config/pet/snippets.toml` (single file) | `~/.config/snp/snippets.toml` (legacy single-file mode) | Supported differently | — | Different paths. snp's default file is pet-compatible TOML. |
| Named libraries | Not supported. Single-file only. | `~/.config/snp/libraries/*.toml` with `~/.config/snp/libraries.toml` tracking metadata. Primary library model. | New | — | snp-only feature. First library created is auto-marked primary. |
| Library config | No equivalent | `libraries.toml` tracks filename, library_id, is_primary, last_sync, server_id per library. | New | — | snp-only. |
| Cross-library search | No equivalent | TUI searches within a single library. `--library` flag selects which library. | New | — | No cross-library search in TUI yet. |
| Migration path | N/A | `LibraryManager::migrate_from_single_file()` moves `snippets.toml` into `libraries/snippets.toml` and registers it as primary. | Planned | R3 | Automatic migration on first library-mode operation. |

### Config paths summary

| Item | Pet path | snp path |
| --- | --- | --- |
| Default snippets | `~/.config/pet/snippets.toml` | `~/.config/snp/snippets.toml` |
| Libraries config | N/A | `~/.config/snp/libraries.toml` |
| Individual libraries | N/A | `~/.config/snp/libraries/<name>.toml` |
| Sync settings | N/A | `~/.config/snp/sync.toml` |
| Premade libraries | N/A | `~/.config/snp/premade/<name>.toml` |

---

## 3. CLI Commands

### 3.1 run / execute

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | `pet run` | `snp run` (alias: `r`) | Supported differently | — | Both open a fuzzy-search TUI for snippet selection, then execute the selected command. |
| Execution mechanism | Runs via user's shell (`$SHELL -c` on Unix, `cmd /C` on Windows) | Runs via `$SHELL -c` (Unix) or `$COMSPEC /C` (Windows). Configurable timeout via `SNP_COMMAND_TIMEOUT` env var (default 300s for output-mode). | Supported differently | — | snp adds configurable timeout and output-to-file when `output` field is set. |
| Output field behavior | Displays output metadata | When `output` field is non-empty, stdout is redirected to a file at that path (relative to CWD). Path traversal protection enforced. | Supported differently | — | snp treats `output` as a file path for stdout redirection, not just metadata display. |
| Variable expansion | Prompts interactively for `<name>` / `<name=default>` | Same syntax. TUI prompts for variables. Supports `\<` `\>` escapes and nested brackets `<a<b>>`. | Full | — | snp is a superset of pet's variable system. |
| Clipboard flag | Not present | `--sync` flag triggers background sync after execution. Copy flag from TUI (`c` key) copies to clipboard instead of executing. | New | — | snp adds TUI copy-mode and post-execution sync. |
| Shell selection | User's `$SHELL` | `$SHELL` on Unix, `$COMSPEC` on Windows. Falls back to `/bin/sh` or `cmd.exe`. | Full | — | Equivalent behavior. |

### 3.2 new / add

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | `pet new` | `snp new` (alias: `n`) | Supported differently | — | Both create snippets interactively. |
| Interactive prompts | Prompts for command, description, tag | Prompts for command, description, tags. Color-coded prompts (yellow/green/cyan). | Full | — | Equivalent workflow. |
| CLI arguments | `pet new [command]` | `snp new [command] [-d description] [-t [tags]] [-m multiline] [--command-stdin] [--from-file] [--editor] [--library lib]` | Supported differently | R2A-R2B | `--tags` without a value still prompts; a value is comma/space-separated. `--command-stdin` is exact UTF-8 ingestion for shell helpers. `--from-file` follows symlinks (resolved target must be a regular file). `--editor` supports `$VISUAL`/`$EDITOR` with arguments via shell-words parsing. |
| Multiline support | `pet new --multiline` | `snp new --multiline` (alias: `-m`). Reads from stdin terminated by two blank lines. | Full | — | Equivalent. Multiline input from stdin. |
| Library target | Single file only | `--library` flag to target a specific library. Falls back to primary library or legacy path. | New | — | snp-only. |
| Config override | N/A | `--config` flag for custom config path. | New | — | snp-only. |
| Duplicate handling | Appends to single file | Appends to library file. IDs auto-generated, deduplicated on load. | Supported differently | — | snp assigns UUIDs; pet has no IDs. |

### Release 2A: Command ingestion and history capture

Release 2A is implemented. It adds `snp new --command-stdin` and generated
`snp_new_current` / `snp_new_previous` helpers for Bash, Zsh, and Fish while
leaving positional `snp new` behavior unchanged.

- stdin is read as exact bytes, then validated as UTF-8; supplied trailing
  newlines are preserved and no newline is appended or trimmed;
- NUL bytes and inputs larger than 16 MiB are rejected before library mutation;
- `--description` is required because command stdin is never reused for
  metadata prompts; use explicit `--tags value` or omit `--tags`;
- helpers use shell-native buffer/history APIs, do not execute or evaluate the
  captured command, do not read history files, and install no keybindings;
- previous-command helpers account for their own invocation so they do not
  capture the helper call as the new snippet;
- shell history can contain credentials, tokens, private URLs, and other
  secrets. Review history before saving and avoid putting secrets in it.

### Release 2B: File and editor creation

Release 2B adds `snp new --from-file` and `snp new --editor`.

- `--from-file` follows symlinks; the resolved target must be a regular file.
  Directories, FIFOs, sockets, and device nodes are rejected. Broken symlinks
  produce an error. Content is stored verbatim with the same validation as
  stdin (16 MiB, UTF-8, no NUL, no empty/whitespace-only).
- `--editor` resolves `$VISUAL` (if set), then `$EDITOR`, then `vim`. The editor
  specification is parsed with `shell-words` so arguments like `code --wait`
  work without invoking a shell. Temp files are created atomically via
  `tempfile::Builder` with `0600` permissions on Unix and RAII cleanup.
- Pet does not have `--from-file` or `--editor` equivalents. These are snp
  extensions for users who prefer external editing workflows.

### 3.3 search

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | `pet search` | `snp search` (alias: `s`) | Supported differently | — | Both open a selection TUI. |
| Search mechanism | External fuzzy finder (`fzf` or `peco`, configured via `pet.config`) | Built-in fuzzy search (fuzzy-matcher crate, skim algorithm). No external dependency. | Supported differently | — | snp has native TUI search; no fzf/peco required. |
| Selection output | Selected snippet's command is inserted into shell buffer (via shell function) | Displays snippet details (description, command, output, tags, folders, favorite) to stdout. | Supported differently | — | snp `search` is inspection-focused. Shell-buffer insertion is planned for R1. |
| Initial query | Shell buffer content passed as query | `--filter` flag provides initial filter string. | Supported differently | — | snp does not yet receive shell buffer as query. R1B adds this. |
| Library scoping | Single file | `--library` flag. Searches within a single library. | New | — | snp-only. |

### 3.4 select

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | No equivalent | `snp select` (alias: `sel`) | New | — | Non-executing selection primitive for shell integration. |
| Output | N/A | Prints selected command to stdout (or `--output-file`). | New | — | Machine-readable output for piping and shell integration. |
| Cancellation | N/A | Exit code 4 on `q`/`Esc`/Ctrl-C. Empty stdout. | New | — | Shell adapters check exit code 4 and restore buffer. |
| Raw vs expanded | N/A | `--raw` (default) prints command unchanged. `--expanded` prompts for variables. Mutually exclusive. | New | — | `--raw` is for placeholder insertion; `--expanded` is for evaluated commands. |
| Initial query | N/A | `--query` (alias `--filter`) pre-fills TUI search. | New | — | Shell adapters pass `$BUFFER` when non-empty. |
| Output file | N/A | `--output-file` writes selection to file. Rejects symlinks and directories. | New | — | Used by shell adapters for lossless multiline transport. |

### 3.5 edit

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | `pet edit` | `snp edit` (alias: `e`) | Supported differently | — | Both open the snippet file in `$EDITOR`. |
| Target file | `~/.config/pet/snippets.toml` | Primary library TOML or `--library` target. Falls back to legacy snippets path. | Supported differently | — | snp opens the active library file. |
| Editor resolution | `$EDITOR` | `$EDITOR`, falls back to `vim`. Resolves bare names via PATH. Symlink-attack protection. | Supported differently | — | snp adds path resolution and security checks. |

### 3.6 list

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | `pet list` | `snp list` (alias: `l`) | Supported differently | — | Both list snippets. |
| Output format | Human-readable to stdout | Three formats: human-readable (default), `--json`, `--csv`. JSON includes all fields. CSV includes description, command, output, tags, folders, favorite. | Supported differently | — | snp adds structured output formats. |
| Filter | Not supported natively | `--filter` flag with fuzzy matching on description+command. | New | — | snp-only. |
| Library scoping | Single file | `--library` flag. `--config` flag for legacy path. | New | — | snp-only. |
| Deleted snippets | No concept | Filtered out by default (tombstoned for sync). | New | — | snp-only. |

### 3.7 sync

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | `pet sync` | `snp sync` (alias: `y`) | Intentional difference | — | Completely different sync backends. |
| Backend | GitHub Gist / GitHub Enterprise Gist / GitLab Snippets | Self-hosted gRPC server (`snip-sync`), encrypted end-to-end. | Intentional difference | — | snp does not use hosted plaintext sync. See Product Invariant #5. |
| Conflict resolution | Last-write-wins on Gist | Last-write-wins based on `updated_at` timestamp. Tombstone propagation for deletions. | Intentional difference | — | Different trust and conflict models. |
| Modes | Push/pull via Git | `--push-only`, `--pull-only`, `--dry-run`, `--servers`. Bidirectional by default. | New | — | snp-only. |
| Encryption | Plaintext on GitHub/GitLab | AES-256-GCM + Argon2id key derivation. API keys hashed with Argon2id. | New | — | snp-only. |
| Auto-sync | `pet` config: `auto_sync = true` | Not yet implemented. Manual/scheduled via `snp cron`. | Planned | R5 | Will add `auto_sync` with debounce. |
| Cron scheduling | Not built-in (user configures externally) | `snp cron --interval <minutes>` sets up automatic sync. | New | — | snp-only. |

### 3.8 import / export

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Import | `pet import` (imports from various sources) | `snp import pet <path>` with `--library`, `--merge`, `--replace`, `--dry-run`, `--strict`, `--report human|json`, `--report-file` options. | Full | R3B | Creates named library from pet file. Source never modified. Atomic writes with backup. Duplicate detection and diagnostics. |
| Export | `pet export` (exports to various formats) | Not implemented. | Under consideration | — | Structured export may be added. Not on current roadmap. |

---

## 4. Variable System

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Required variable | `<name>` — prompts user, no default | Same syntax. Parsed by `utils/variables.rs::parse_variables()`. | Full | — | Equivalent. |
| Default variable | `<name=default>` — prompts user, accepts default on Enter | Same syntax. Default value shown in prompt. | Full | — | Equivalent. |
| Escaped brackets | Not supported | `\<` and `\>` produce literal `<` and `>`. Parsed by `strip_escape_sequences()`. | New | — | snp-only extension. |
| Nested brackets | Not supported | `<a<b>>` — outer variable `a` with default `b>`. | New | — | snp-only extension. |
| Variable prompting | CLI prompt (text-based) | TUI-based prompt with color-coded input. Supports Cancel, Back, Skip. | Supported differently | — | snp has richer TUI prompting. |
| Multiple-choice defaults | `<param=\|_opt1_\|\|_opt2_\|\|_opt3_\|\|>` syntax | Recognized and prompted as a list selector. Raw text preserved in storage. | Full | R3A | Choice syntax parsed into `VariableKind::Choices`. First choice is default. |
| Chained variables | `<a><b>` — two adjacent variables | Supported. Each variable prompted independently. | Full | — | Equivalent. |
| Variables in quoted shell text | Standard behavior | Supported. Variables work inside quotes. | Full | — | Equivalent. |

### Edge cases

| Case | Pet behavior | snp behavior |
| --- | --- | --- |
| Unmatched `<` without `>` | Treated as literal `<` | Treated as literal `<` — no variable substitution, character preserved. |
| Empty default `<name=>` | Prompts with empty default | Same — empty default accepted. |
| Unicode variable values | Supported | Supported — UTF-8 throughout. |

---

## 5. Clipboard Operations

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Command | `pet copy` | `snp clip` (alias: `c`) | Supported differently | — | Equivalent workflow. |
| Clipboard library | OS clipboard (platform-specific) | `arboard` crate — cross-platform (macOS, Linux, Windows). | Full | — | snp uses a maintained cross-platform crate. |
| Copy target | Selected snippet's command (expanded) | Expanded command (after variable prompting). Clipboard via `copy_to_clipboard_auto()`. | Full | — | Equivalent behavior. |
| TUI integration | Separate command | `snp clip` opens TUI, press `c` to copy. `snp run` also supports copy via TUI `c` key. | Supported differently | — | snp integrates copy into both `run` and `clip` commands. |

---

## 6. Tag / Folder Organization

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Tags | `Tag = ["git", "version-control"]` | `tag = ["git", "version-control"]`. Reads `Tag`, `Tags`, `tags` aliases. | Full | — | Equivalent. snp reads all casing variants. |
| Folders | Not supported | `folders = ["work", "scripts"]` — organizational grouping. | New | — | snp-only. |
| Tag filtering | Via external fuzzy finder | TUI filter matches on description+command text (not tags directly). | Supported differently | — | Tag-based filtering is not yet first-class in the TUI filter. |
| Favorite flag | Not supported | `favorite = true` — marks snippet as favorite. | New | — | snp-only. |

---

## 7. Shell Integration

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Shell init script | `pet init bash` / `pet init zsh` / `pet init fish` — generates shell functions | `snp shell init bash/zsh/fish` — generates `snp_select_raw` and `snp_select_expanded` functions. | Full | — | Shell adapters use temp-file transport and exit code 4 for cancellation. |
| Shell completions | Built-in fish completions. Bash/zsh via init script. | Via `clap_complete`: `snp completions bash/zsh/fish/powershell/elvish`. | Supported differently | — | snp uses clap's completion generator. No init script. |
| Buffer insertion | Shell function wraps `pet search`, inserts selected command into shell buffer | `snp_select_raw` and `snp_select_expanded` call `snp select --output-file <tmpfile>`, read back, and set `$BUFFER`/`READLINE_LINE`. | Full | — | Lossless multiline handling via temp-file transport. |
| Keybinding | Shell function bound to Ctrl+R by default | Not installed by default. Generated code includes binding examples in comments. | Supported differently | — | Opt-in. User binds `snp_select_raw`/`snp_select_expanded` manually. |
| Current buffer as query | Shell function passes `$BUFFER` to `pet search --query` | `snp select --query "$BUFFER"` (or `--filter`) passes current buffer as initial search query. | Full | — | Shell adapters pass buffer when non-empty. |

---

## 8. Search / Filter

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Search engine | External: `fzf`, `peco`, or custom (via `pet.config`) | Native: `fuzzy-matcher` crate (skim algorithm). Built into TUI. | Supported differently | — | snp has zero external dependencies for search. |
| Fuzzy matching | Delegated to external tool | Built-in skim algorithm. Debounced filter updates (150ms). | Supported differently | — | Equivalent user experience, different implementation. |
| TUI interaction | N/A (external tool handles UI) | Full TUI with Vim keybindings, preview, sorting, folder/favorite filtering. | New | — | snp-only rich TUI. |
| Sorting | Configurable via pet config | `SortMode` enum in `ui/state.rs`. Relevance-based (fuzzy score). | Partial | R4A | Optional sort modes (recent, last-used, most-used, description, command) planned for R4. |
| Initial query | Shell buffer content | `--filter` flag. No shell buffer integration yet. | Partial | R1B | Will accept initial query from shell buffer. |

---

## 9. Data Migration

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Direct file loading | N/A | Pet TOML files load directly in snp — same `[[snippets]]` table, same field names. | Full | — | No conversion needed for basic migration. |
| Explicit import command | `pet import` | `snp import pet <path>` with diagnostics, dry-run, merge, replace, strict, and report options. | Full | R3B | Creates named library from pet file. Source never modified. Atomic writes with backup. |
| Diagnostic tool | Not built-in | `snp doctor --pet-file <path>` for read-only compatibility diagnostics, `snp doctor --compatibility` for environment audit. | Full | R3C | Human and JSON reports. Uses shared analysis with import. |
| Migration strategy | N/A | 1. Run `snp doctor --pet-file <path>` to check compatibility before importing. 2. Copy `~/.config/pet/snippets.toml` to `~/.config/snp/libraries/pet-snippets.toml`. 3. Or use `snp import pet <path>` for explicit migration with report. | Full | R3B | Recommended workflow: doctor first, then import or manual copy. |
| Metadata loss | N/A | Pet files lose no data on import. snp adds IDs, timestamps, and device_id automatically. | Full | — | Forward-compatible — pet ignores unknown fields. |
| Round-trip editing | N/A | If both pet and snp edit the same file, snp-only metadata (IDs, timestamps, folders, favorites) may be lost when pet writes the file. | Documented | — | Documented caveat. Not recommended for concurrent editing. |

---

## 10. Sync

| Aspect | Pet behavior | snp current behavior | Compatibility status | Release target | Notes |
| --- | --- | --- | --- | --- | --- |
| Sync backend | GitHub Gist / GitHub Enterprise Gist / GitLab Snippets | Self-hosted gRPC server (`snip-sync`). Encrypted with AES-256-GCM + Argon2id. | Intentional difference | — | Different trust model. snp does not use hosted plaintext sync. |
| Multi-device | Via Git push/pull | Via gRPC. Device ID tracking. Last-write-wins conflict resolution. | Intentional difference | — | Equivalent goal, different mechanism. |
| Encryption | Plaintext (GitHub/GitLab handle transport encryption) | End-to-end encryption. API keys hashed with Argon2id. | New | — | snp-only security feature. |
| Auto-sync | `auto_sync = true` in pet config | Not yet. `snp cron` provides scheduled sync. Manual sync via `snp sync`. | Planned | R5 | Will add auto-sync with debounce and failure handling. |
| Server management | N/A | `snp register` creates account. `snp sync --servers` lists connected servers. | New | — | snp-only. |
| Premade libraries | Not supported | `snp premade list/get/sync/search/update` — browse and download community snippet collections from server. | New | — | snp-only feature. |

---

## 11. Additional Features (snp-only)

These features have no pet equivalent and represent snp's native capabilities:

| Feature | Description | Release |
| --- | --- | --- |
| Themes | Halloy-compatible TOML themes. 50 bundled themes. `ThemeManager` with live preview. | Current |
| Premade libraries | Browse/download community snippet collections from sync server. | Current |
| Pet import | `snp import pet <path>` — import pet snippet files with diagnostics, dry-run, merge, and JSON reports. | R3B |
| Compatibility diagnostics | `snp doctor` — read-only pet file analysis and installed environment audit with human/JSON reports. | R3C |
| Backup & recovery | Automatic timestamped backups before saves (max 10 per library). | Current |
| Audit log | `~/.config/snp/audit.log` — tracks snippet operations (create, execute, copy, delete). | Current |
| Structured output | `snp list --json` / `snp list --csv` for scripting and spreadsheet import. | Current |
| Shell completions | Via clap_complete for bash, zsh, fish, powershell, elvish. | Current |
| Self-update | `snp update` — check for and install updates using current installation method. | Current |
| Keybinding reference | `snp keybindings` — display TUI keybindings. | Current |
| Configurable timeout | `SNP_COMMAND_TIMEOUT` env var for command execution timeout. | Current |

---

## Release Roadmap Summary

| Release | Focus | Key deliverables |
| --- | --- | --- |
| R1A | Compatibility contract & baseline | This matrix, regression tests, fixtures, architecture inventory |
| R1B | Machine-facing selection primitive | `snp select` command with stdout/exit-code contract |
| R1C | Shell-buffer integration | `snp shell init bash/zsh/fish` with buffer-insertion functions |
| R2A | Shell history capture | `snp new --command-stdin`, save previous/current buffer |
| R2B | Multiline & editor creation | `snp new --multiline --command-stdin --from-file --editor` |
| R3A | Pet multiple-choice parameters | Variable parser recognizes `Choice\|opt1\|opt2` syntax |
| R3B | Explicit pet import | `snp import pet <path>` with diagnostics and merge options |
| R3C | Compatibility diagnostics | `snp doctor --pet-file <path>` and `snp doctor --compatibility` — shared diagnostic model with import |
| R4A | Optional sorting | `--sort relevance/recent/last-used/most-used/description/command` |
| R4B | Output & notes presentation | Expose `output` field in preview, editing, export |
| R5 | Auto-sync | `auto_sync = true` with debounce and failure handling |

---

## Product Invariants

These invariants must be preserved across all releases:

1. **Existing behavior is frozen** unless separately approved. All current `snp` command semantics remain unchanged.
2. **Compatibility features are additive and opt-in.** No silent shell bindings, no rewritten startup files, no changed defaults.
3. **Selection and execution remain distinct.** Shell-buffer selection emits text; it never executes.
4. **Native snp architecture remains primary.** No fzf/peco as required dependencies. No external selector abstraction.
5. **Synchronization remains security-oriented.** No GitHub Gist/GitLab backends. `snip-sync` is canonical.
6. **Source compatibility is stronger than round-trip identity.** Pet TOML loads without conversion; concurrent editing of the same file by both tools is not supported when snp metadata is present.
