# UI Components (`ui/` theme, highlight, variables)

[← Back to Overview](overview.md)

## Overview

`src/ui/` (6 files) splits TUI duties: `mod.rs` owns the event loop (see
[tui.md](tui.md)), while `theme.rs`, `highlight.rs`, and `variables.rs`
own appearance, syntax coloring, and the variable-prompt dialog.
`_generated_bundled_themes.rs` is build output — **read only, never edit**.
The module's public surface is exactly two re-exports; widening it breaks
every TUI command (AGENTS pitfall #5).

## Module map

| File | Lines | Purpose |
|------|-------|---------|
| `mod.rs` | 2063 | Event loop; re-exports `theme::get_theme`, `variables::{VariablePromptResult, prompt_variables}` (`:16`) |
| `state.rs` | 250 | `SelectState` / `FilterState` / `SortMode` (see [tui.md](tui.md)) |
| `theme.rs` | 1063 | `Theme` 10-color palette, Halloy parsing, `ThemeManager`, global active theme |
| `highlight.rs` | 248 | `highlight_command()` tokenizer |
| `variables.rs` | 1579 | Modal `<name>` / `<name=default>` prompt dialog |
| `_generated_bundled_themes.rs` | 301 | Generated bundle (50 themes); regen via `python3 scripts/build_themes.py` |

## Theme: 10-color palette (`src/ui/theme.rs`)

`Theme` (`:26`) is `Clone + Copy` so the draw closure snapshots it cheaply:

`primary`, `secondary`, `accent`, `background`, `text`, `border`,
`selected_bg`, `muted`, `string_color`, `escape_color`.

- `DARK_THEME` (`:41`) — last-resort fallback; legacy `SNP_THEME=dark` / `COLORFGBG` path.
- `BRIGHT_THEME` (`:55`) — legacy `SNP_THEME=bright|light`.
- Halloy schema (`:82`): private `HalloyTheme`/`HalloyGeneral`/`HalloyText`/`HalloyBuffer`/`HalloyButtons`/`HalloyFormatting` structs mirror the post-2024.11 Halloy `Styles`; all `#[serde(default)]` so inconsistent real-world themes parse gracefully. Colors deserialize as `String` then convert (`Color` has no `Deserialize` impl).
- `parse_halloy_string` (`:730`) → `into_snp_theme()` projects Halloy onto the 10 colors.
- `style_fg` / `style_fg_bg` (`:739`, `:743`) are the only style constructors; highlight and rendering go through them.
- Active theme is process-global: `ACTIVE_THEME: LazyLock<RwLock<Theme>>` (`:639`); `get_theme()` (`:643`) is one read-lock + `Copy`; `set_active_theme()` (`:651`) publishes picker previews/commits for the next draw frame.
- Resolution order (`load_initial_theme`, `:663`): (1) `themes.toml` active name → `themes/` file (`load_from_themes_toml`, `:682`); (2) `SNP_THEME` (`dark`/`bright`/`light`/`auto` consts, else a filename in `themes/`, `:697`); (3) hardcoded `DEFAULT_BUNDLED`; (4) `DARK_THEME` safety net. `auto` consults `COLORFGBG` (`:702`).
- `validate_theme_name` (`:605`) rejects empty/over-100-char names, slashes, NULs, `..`, and control characters — mirroring the build script's filename gate.

## `ThemeManager` + 50 Halloy themes

- `ThemesConfig` (`:389`): `{ active: Option<String> }` basename persisted in `~/.config/snp/themes.toml`. `ThemeInfo` (`:398`): `name`, `path`, `is_bundled`.
- `ThemeManager` (`:416`): rooted at the config dir; `new()` (`:434`) reads `themes.toml` with warn-and-default on missing/unparseable files and never touches `themes/` itself.
- `init_themes_dir()` (`:477`): `create_dir_all`, write `DEFAULT_BUNDLED` first (so first launch has Cyber Red on disk), then decode every bundled theme. **Idempotent** — `write_theme_if_missing` (`:592`) never overwrites, so user edits to seeded themes survive upgrades.
- `list_themes()` (`:497`) returns `.toml` files sorted by name; `load_theme(name)` (`:527`) parses one file; `set_active_theme(name)` (`:544`) persists the basename.
- Bundled-name check (`:575`) treats `"Cyber Red"` (`bundled_default_name`, `:587`) plus every `BUNDLED` entry as bundled.

## Bundling pipeline (never hand-edit output)

1. Authors edit `themes/*.toml` (50 files).
2. `python3 scripts/build_themes.py` (`scripts/build_themes.py:10`): `collect_themes` sorts for determinism (`:66`); each non-default theme is gzip-compressed (`compress_gzip`, `:54`, level 6, `mtime=0`, matching `flate2::GzDecoder`) and base64-encoded; filenames are gated by `FILENAME_RE` against path traversal (`:42`); `Cyber Red.toml` is embedded uncompressed as `DEFAULT_BUNDLED` (`:139`, `:156`); `BUNDLED: &[BundledTheme]` holds `(name, payload_b64)` sorted case-insensitively (`:98`, `:171`); `bundled_themes_decoded()` (`:196`, emitted `:280` in the generated file) decodes on demand.
3. Output header (`_generated_bundled_themes.rs:1`) marks provenance and the regen command; `#![allow(dead_code, unused_imports)]` (`:13`) covers test-vs-release use.
4. Normal Cargo builds consume the checked-in file; the script runs only after theme edits. `ThemeManager::init_themes_dir` extracts the bundle once per user (never in hot paths — ~50 KiB of gzip per call).

## Highlight (`src/ui/highlight.rs`)

`highlight_command(command)` (`:38`) returns one `Line<'static>` of theme-colored spans:

| Token | Color | Rule (`highlight.rs`) |
|-------|-------|----------------------|
| Variables `<…>` | `accent` | Only when a closing `>` exists ahead (`gt_remaining` suffix counts, `:44`); unclosed `<` is literal so the rest of the line keeps normal styling (`:80`); `\<`/`\>` escape to default (`:65`) |
| Shell keywords | `primary` | Whole-word lookup in `SHELL_KEYWORDS_SET` (`:161`), `src/utils/shell_keywords.rs:4` (`SHELL_KEYWORDS`, ~190 names) materialized into a `HashSet` (`:197`) |
| Strings | `string_color` | `'...'` / `"..."` runs (`:113`) |
| Flags | `secondary` | `-` followed by `[alnum-=]` runs (`:125`) |
| Comments | `muted` | `#` starting a whitespace-led span consumes the rest (`:102`) |
| Escapes | `escape_color` | `\x` pairs and a preserved trailing lone `\` (`:169`) |
| Default/whitespace | `text` | Words (`:148`) and whitespace runs (`:136`) |

`CountingChars` (`:15`) tracks consumed offsets without rescanning. Edge cases are pinned by tests: unclosed variable renders literally (`:199`) yet `--flag` after it still tokenizes (`:206`); trailing backslash is preserved (`:220`).

## Variables TUI (`src/ui/variables.rs`)

`prompt_variables(vars)` (`:78`) returns `Skip` immediately for empty input; otherwise `prompt_variables_inner` runs a modal dialog returning `VariablePromptResult` (`:56`): `Cancel` (Ctrl+C / signal — exit the program), `Back` (`q` — return to the selector), `Skip` (nothing to fill), `Values(Vec<(String, String)>)`.

- Syntax is `<name>` / `<name=default>` (`Variable`, `VariableKind` in `src/utils/variables.rs`); unmatched `<` without `>` is literal (see `has_unmatched_angle_bracket`, tui.md edge cases).
- Dual editing modes (module docs, `:6`): **INS** default inserts literally (so `q` types "queue"); **NOR** offers `h`/`l` cursor, `j`/`k` variable jumps, `d` restore-default, `q` back, `i` return to INS. Only Ctrl+C exits the program.
- `Field` (`:87`) holds `value` + a byte-index cursor kept on char boundaries (`insert_char`, `backspace`, `delete`, `:101`); own `TerminalGuard` (`:41`) mirrors the main loop's restore guarantee.
- Call path: `expand_snippet_command` (`src/commands/mod.rs`) calls `prompt_variables()` before shell expansion.

## Re-export surface (AGENTS pitfall #5)

`src/ui/mod.rs:16` re-exports exactly `theme::get_theme` and
`variables::{VariablePromptResult, prompt_variables}`; `src/commands/mod.rs`
(`load/save_snippets`, `run_snippet_selection`) has the same blast radius.
Adding, removing, or renaming these items — or the `ui::SnippetListParams` /
`SnippetSelection` types — touches all TUI commands (`run`, `clip`,
`search`, `select`). Extend inside `theme.rs` / `variables.rs` instead.

## First-launch seeding sequence

1. `ThemeManager::new()` (`theme.rs:434`) reads `themes.toml` (absent → defaults).
2. `init_themes_dir()` (`:477`) creates `themes/`, writes `DEFAULT_BUNDLED` ("Cyber Red") first, then every decoded bundle entry — skipping existing files.
3. `load_initial_theme()` (`:663`) resolves (1) persisted name → (2) `SNP_THEME` → (3) bundled default → (4) `DARK_THEME`.
4. The selector snapshots the theme per draw via `get_theme()`; the picker publishes previews into `ACTIVE_THEME` without persisting until commit.

## Theme-picker interaction

| Key | Action |
|-----|--------|
| `j` / `k` (arrows) | Move and live-preview |
| `i` | Filter themes by name (fuzzy, 100 ms debounce) |
| `Enter` | Persist selection to `themes.toml` |
| `e` / `q` / `Esc` | Cancel and restore the pre-picker theme |

Re-highlight is scheduled (`pending_rehighlight`, `mod.rs:611`) so the new
palette applies to cached command rows on the next frame.

## Invariants

- Generated bundle is never edited by hand; theme edits go to `themes/` + regen.
- Seeded user themes are never overwritten (`write_theme_if_missing`).
- `get_theme()` stays lock-then-`Copy` cheap; no I/O in the draw path.
- Highlight output is lossless: concatenated span contents always equal the input command.
- Variable `Back`/`Cancel`/`Skip` are distinct control signals — callers must not conflate "back to selector" with "exit program" or "no variables".

## Key files

- `src/ui/theme.rs:26` (`Theme`), `:416` (`ThemeManager`), `:639` (`ACTIVE_THEME`), `:739` (style helpers)
- `src/ui/highlight.rs:38` (`highlight_command`), `src/utils/shell_keywords.rs:4`
- `src/ui/variables.rs:56`, `:78`
- `src/ui/_generated_bundled_themes.rs` (generated — read only), `scripts/build_themes.py`, `themes/` (50 sources)
