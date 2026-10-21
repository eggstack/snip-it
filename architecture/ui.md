# UI Presentation (`src/ui/` theme, highlight, variables)

[← Back to Overview](overview.md)

## Overview

This doc covers the *presentation* half of the `src/ui/` module: the theme
palette and its on-disk format, theme discovery and precedence, the bundled
theme pipeline, and the shell syntax highlighter.

The interactive event loop that consumes all of this — keyboard map, filter
debouncing, layout, selection state — lives in [tui.md](tui.md). The
variable-prompt dialog is also part of the TUI and is described there
([Variable prompt flow](tui.md#variable-prompt-flow-srcuivariablesrs)); this
doc only covers the *syntax* it consumes. Variable parsing and expansion
themselves live in `src/utils/variables.rs` and are documented in
[utils/variables.md](utils/variables.md); the shell command-name list lives in
`src/utils/shell_keywords.rs` and is documented in
[utils/shell_keywords.md](utils/shell_keywords.md).

## Module map

`src/ui/mod.rs:9-17` declares the module tree:

```rust
// src/ui/mod.rs:9-17
mod highlight;
mod state;
mod theme;
mod variables;

mod _generated_bundled_themes;

pub use theme::get_theme;
pub use variables::{VariablePromptResult, prompt_variables};
```

| Module | Lines | Declared items | Role |
|--------|-------|----------------|------|
| `src/ui/theme.rs` | 1063 | `Theme`, `ThemesConfig`, `ThemeInfo`, `ThemeManager`, `get_theme`, `set_active_theme`, `style_fg`, `style_fg_bg` | Palette, Halloy TOML parsing, theme directory management, active-theme persistence |
| `src/ui/highlight.rs` | 248 | `highlight_command` (`pub(crate)`) | Shell-command tokenizer → styled `ratatui::text::Line` |
| `src/ui/variables.rs` | 1579 | `prompt_variables`, `VariablePromptResult` | Modal variable prompt — see [tui.md](tui.md) |
| `src/ui/state.rs` | 250 | *(all `pub(super)`)* | `SelectState`, `FilterState`, `SortMode` — see [tui.md](tui.md) |
| `src/ui/_generated_bundled_themes.rs` | 301 | `DEFAULT_BUNDLED`, `BundledTheme`, `BUNDLED`, `bundled_themes_decoded` | **Generated.** Never hand-edit |
| `src/utils/shell_keywords.rs` | 198 | `SHELL_KEYWORDS`, `SHELL_KEYWORDS_SET` | 190 shell command names for the highlighter |
| `src/utils/variables.rs` | 1947 | `parse_variables`, `expand_command`, `strip_escape_sequences`, … | Variable syntax — see [utils/variables.md](utils/variables.md) |

Every module in `src/ui/` is declared private (`mod`, not `pub mod`,
`src/ui/mod.rs:9-14`), so only the three re-exported items at
`src/ui/mod.rs:16-17` (`get_theme`, `VariablePromptResult`,
`prompt_variables`) are reachable from outside `src/ui/` — even though
`src/lib.rs:49` declares `pub mod ui;`. The `pub` on `Theme`,
`ThemeManager`, `prompt_variables`, etc. is module-local in practice. The
two `src/utils/` modules in the table are behind `pub(crate) mod utils;`
(`src/lib.rs:70`), so their `pub` items are crate-internal too.

Only `get_theme` and the two variable-prompt items are re-exported from
`src/ui/mod.rs`; everything else is crate-internal.

## The `Theme` palette

`Theme` is a flat, `Copy` struct of exactly **10** `ratatui::style::Color`
fields (`src/ui/theme.rs:25-37`):

```rust
// src/ui/theme.rs:25-37
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub primary: Color,
    pub secondary: Color,
    pub accent: Color,
    pub background: Color,
    pub text: Color,
    pub border: Color,
    pub selected_bg: Color,
    pub muted: Color,
    pub string_color: Color,
    pub escape_color: Color,
}
```

There are **no** `Option`s: every field is always a usable colour. That is
guaranteed by `first_some_color` (`src/ui/theme.rs:373-380`), which walks a
fallback chain and returns `Color::White` when every candidate is missing.

Two built-in palettes exist for the legacy paths (`src/ui/theme.rs:41-66`):
`DARK_THEME` (`SNP_THEME=dark`) and `BRIGHT_THEME` (`SNP_THEME=bright|light`).

### Where each colour is used

| Field | Used for | Consumer |
|-------|----------|----------|
| `primary` | shell keywords, filter box border | `src/ui/highlight.rs:56`, `src/ui/mod.rs` filter block |
| `secondary` | command-line flags | `src/ui/highlight.rs:58` |
| `accent` | `<variable>` placeholders | `src/ui/highlight.rs:55` |
| `background` | list / block background | `src/ui/mod.rs` list rendering |
| `text` | default text, whitespace, plain words | `src/ui/highlight.rs:54` |
| `border` | block borders | `src/ui/mod.rs` |
| `selected_bg` | highlighted row background | `src/ui/mod.rs` list rendering |
| `muted` | comments, secondary text | `src/ui/highlight.rs:59` |
| `string_color` | quoted strings | `src/ui/highlight.rs:57` |
| `escape_color` | backslash escape sequences | `src/ui/highlight.rs:60` |

### Halloy → `Theme` projection

Themes on disk are [Halloy](https://halloy.dev)-style TOML files. The on-disk
schema is mirrored privately by `HalloyTheme` (`src/ui/theme.rs:82-88`) and its
six sub-structs (`HalloyGeneral` `:92`, `HalloyText` `:104`, `HalloyBuffer`
`:118`, `HalloyServerMessages` `:142`, `HalloyButtons` `:148`,
`HalloyFormatting` `:165`). Every field is `#[serde(default)]`, so unknown
sections and missing fields parse gracefully — the community themes on disk are
inconsistent.

Colours are deserialized as `String` and converted afterwards, because
`ratatui::style::Color` has no `serde::Deserialize` implementation
(`src/ui/theme.rs:76-78`). `HalloyTextStyle` (`:186-191`) accepts both the
basic `"#RRGGBB"` form and the extended `{ color = "#RRGGBB", font_style =
"bold" }` form via an untagged enum (`:193-214`).

`into_snp_theme` (`src/ui/theme.rs:338-368`) projects the rich schema onto the
10 fields using short fallback chains, so a partial theme still yields a
coherent palette:

| `Theme` field | Chain (first non-`Reset` wins) |
|---------------|-------------------------------|
| `primary` | `text.primary` → `text.tertiary` → white |
| `secondary` | `text.tertiary` → `text.secondary` → white |
| `accent` | `text.tertiary` → `text.success` → white |
| `background` | `general.background` → `buffer.background` → white |
| `text` | `text.primary` → white |
| `border` | `general.border` → `text.tertiary` → white |
| `selected_bg` | `buffer.highlight` → `buffer.selection` → white |
| `muted` | `buffer.timestamp` → `text.secondary` → white |
| `string_color` | `formatting.green` → `text.success` → white |
| `escape_color` | `formatting.magenta` → `text.error` → white |

`Color::Reset` is treated as "missing" and walks the chain; the final
fallback is always `Color::White`, never `Reset`.

Hex parsing is `hex_to_color` (`src/ui/theme.rs:303-321`): a leading `#` is
stripped and the remainder must be **6** hex digits (`#RRGGBB`) or **8**
hex digits (`#RRGGBBAA`, whose alpha channel is discarded), producing
`Color::Rgb(r, g, b)`. Anything else yields `None`, and the empty string is
treated as "missing" so it falls through the chain.

## Theme discovery and precedence

### The on-disk layout

```text
~/.config/snp/
├── themes.toml          ← active-theme selection
└── themes/
    ├── Cyber Red.toml
    ├── Catppuccin Mocha.toml
    └── … (50 files)
```

`themes/` is the user's theme directory; `themes.toml` records which one is
active. Both live under the config dir returned by
`crate::utils::config::get_config_dir()` (`src/ui/theme.rs:435-437`).

### Initial resolution order

`load_initial_theme` (`src/ui/theme.rs:663-680`) is called once, lazily, when
the process-global `ACTIVE_THEME` is first touched (`src/ui/theme.rs:639`).
It tries four sources in order:

| # | Source | Behaviour | Location |
|---|--------|-----------|----------|
| 1 | `themes.toml` → `[active] name` | Reads `themes/<name>.toml` and parses it | `:664-667`, `load_from_themes_toml` `:682-695` |
| 2 | `SNP_THEME` env var | `"dark"` → `DARK_THEME`; `"bright"`/`"light"` → `BRIGHT_THEME`; `"auto"` → picks by `COLORFGBG`; **any other value is treated as a theme filename** relative to `themes/` | `:668-673`, `resolve_legacy_or_filename` `:697-728` |
| 3 | `DEFAULT_BUNDLED` | The embedded `Cyber Red` TOML, parsed in memory — no file needed | `:674-677` |
| 4 | `DARK_THEME` | Last-resort safety net | `:678-679` |

Every step is fallible and falls through silently; a broken `themes.toml`
never prevents startup.

`SNP_THEME` values containing `/`, `\`, `..`, or NUL are rejected with a
warning before the lookup (`:710-720`) — the same path-traversal defence as
`validate_theme_name`.

### Runtime switching

The active theme is a **process-global** `LazyLock<RwLock<Theme>>`
(`src/ui/theme.rs:639`). Two functions expose it:

```rust
// src/ui/theme.rs:643
pub fn get_theme() -> Theme {
    *ACTIVE_THEME.read().unwrap_or_else(|e| e.into_inner())
}

// src/ui/theme.rs:651
pub fn set_active_theme(theme: Theme) {
    *ACTIVE_THEME.write().unwrap_or_else(|e| e.into_inner()) = theme;
}
```

`get_theme()` is cheap (one read-lock plus a 10-field `Copy`) and is the only
thing the draw loop touches — the theme picker swaps the value in place and
the next frame picks up the new colours.

Both locks are **poison-tolerant**: `unwrap_or_else(|e| e.into_inner())`
recovers the inner value instead of panicking.

### Bundled themes are seeded lazily

`ThemeManager::init_themes_dir` (`src/ui/theme.rs:477-494`) extracts the
bundled themes into `~/.config/snp/themes/`. It is **not** called at startup —
its only caller is `ensure_theme_picker_ready` (`src/ui/mod.rs:540`), which
runs the first time the user opens the theme picker. Consequences:

- On a fresh install the `themes/` directory does not exist until the picker
  is opened.
- The first launch still gets a usable theme, because step 3 of the initial
  resolution parses the embedded `DEFAULT_BUNDLED` string directly.
- `init_themes_dir` is **idempotent**: `write_theme_if_missing`
  (`src/ui/theme.rs:592-599`) skips any file that already exists, so user
  edits to a seeded theme survive upgrades.

### `ThemeManager`

`ThemeManager` (`src/ui/theme.rs:416-425`) owns the three paths plus the
parsed `ThemesConfig`. It is created by `ThemeManager::new()`
(`:434-471`), which reads `themes.toml` if present and **falls back to
defaults on a read or parse error** (logging a warning, `:443-459`) — a
corrupt `themes.toml` never blocks the TUI.

| Method | Location | Behaviour |
|--------|----------|-----------|
| `new()` | `:434-471` | Reads `themes.toml`; does **not** touch `themes/` |
| `init_themes_dir()` | `:477-494` | Creates `themes/` and seeds it from the bundle; idempotent |
| `list_themes()` | `:497-524` | Every `*.toml` in `themes/`, sorted case-insensitively by name; sets `is_bundled` |
| `load_theme(name)` | `:527-536` | Validates the name, parses the file, projects to `Theme` |
| `get_active_theme_name()` | `:539-541` | The `themes.toml` value |
| `set_active_theme(name)` | `:544-549` | Validates, records, and persists atomically |
| `themes_dir()` | `:552-554` | The themes directory path |
| `theme_path(name)` (private) | `:556-558` | `themes/<name>.toml` |
| `save_config()` (private) | `:560-569` | Atomic write of `themes.toml` |

`ThemeInfo` (`:397-407`) carries `name` (basename without `.toml` — the
persistent identifier), `path`, and `is_bundled`.

### Name validation

`validate_theme_name` (`:605-633`) rejects empty names, names longer than
100 characters, names containing `/`, `\`, or NUL, names containing `..`, and
names containing control characters. Spaces, parentheses, dots, dashes, and
underscores are allowed, because the Halloy community themes use names like
`Catppuccin Mocha` and `Discord (80_ Saturation)`.

### `themes.toml`

```toml
active = "Cyber Red"
```

The file is `ThemesConfig` (`src/ui/theme.rs:388-393`) with a single
`active: Option<String>` field. It is written with
`crate::utils::atomic::atomic_write_bytes(..., Durability::SensitiveConfig)`
(`:560-569`), so on Unix it gets `0o600` permissions and symlinks are
rejected. The value is the theme **basename**, not a path.

Note: `themes.toml` is *not* the same file as `themes/*.toml` — the former is
the selection index, the latter are the theme definitions.

### The theme picker

The picker overlay lives in the TUI loop (`src/ui/mod.rs:529-612` and
`:1368-1554`); see [tui.md](tui.md#theme-picker). It:

- snapshots the in-memory `Theme` before opening (`theme_picker_original`),
  so cancelling restores the *rendered* theme even when nothing was ever
  persisted (`:481-497`);
- live-previews on every `j`/`k` by calling `ThemeManager::load_theme` +
  `theme::set_active_theme` (`:440-463`);
- persists only on `Enter` via `commit_theme_picker` (`:465-479`).

## The bundled-theme pipeline

```text
themes/*.toml  (50 files)
      │
      │  python3 scripts/build_themes.py
      ▼
src/ui/_generated_bundled_themes.rs
      │
      │  ThemeManager::init_themes_dir()
      ▼
~/.config/snp/themes/*.toml
```

`scripts/build_themes.py` is the **only** legitimate writer of
`src/ui/_generated_bundled_themes.rs`.

### What the script does

| Step | Detail |
|------|--------|
| Collect | Every `themes/*.toml`, sorted by name for determinism (`collect_themes`) |
| Validate | Filenames must match `^[\w ()\-.,]+\.toml$` (`FILENAME_RE`, `validate_filename`) — permissive enough for community names, strict enough to reject traversal |
| Split | `themes/Cyber Red.toml` becomes the uncompressed `DEFAULT_BUNDLED`; every other theme is gzipped (level 6, `mtime=0`) and base64-encoded |
| Sort | The bundled list is sorted case-insensitively for stable output across machines |
| Emit | `DEFAULT_BUNDLED` as a Rust raw string literal with an auto-escaped `#` delimiter, plus `pub const BUNDLED: &[BundledTheme]` and `bundled_themes_decoded()` |

The script is **not** invoked by Cargo. Normal builds consume the checked-in
generated file; the script must be run explicitly after editing a theme:

```bash
python3 scripts/build_themes.py
```

### The generated file

```rust
// src/ui/_generated_bundled_themes.rs:69-74
#[derive(Debug, Clone, Copy)]
pub struct BundledTheme {
    pub name: &'static str,
    pub payload_b64: &'static str,
}
```

- `DEFAULT_BUNDLED` (`:25`) — the `Cyber Red` theme as a raw string literal.
- `BUNDLED` (`:77-274`) — the other **49** themes, each a base64-encoded
  gzip payload, sorted by name.
- `bundled_themes_decoded()` (`:280-301`) — base64-decodes then
  `flate2::read::GzDecoder`-inflates every theme into `(name, toml_text)`
  pairs. It is documented as **not** for hot paths: each call decodes roughly
  50 KiB of gzip data.

`is_bundled_theme_name` (`src/ui/theme.rs:574-581`) checks a name against
`BUNDLED` plus the hardcoded default name `"Cyber Red"`
(`bundled_default_name`, `:587-589`).

## Syntax highlighting

`src/ui/highlight.rs` implements `highlight_command`
(`:38-174`), which turns a shell command string into a styled
`ratatui::text::Line<'static>`.

```rust
// src/ui/highlight.rs:38
pub(crate) fn highlight_command(command: &str) -> Line<'static>;
```

It is a **single-pass character scanner**, not a shell parser. It recognises
six highlighted token classes — plus two neutral fallbacks (plain word,
whitespace):

| Class | Trigger | Colour | Location |
|-------|---------|--------|----------|
| Escape sequence | `\` followed by any char other than `<` / `>` | `escape_color` | `:65-72` |
| Variable | `<…>` with a matching `>` | `accent` | `:80-100` |
| Comment | `#` at the start of a token | `muted` | `:102-112` |
| String | `"…"` or `'…'` (runs to the closing quote) | `string_color` | `:113-123` |
| Flag | `-` followed by alphanumerics, `-`, or `=` | `secondary` | `:124-135` |
| Keyword | A word present in `SHELL_KEYWORDS_SET` | `primary` | `:161-163` |
| Plain word | Anything else | `text` | `:162` |
| Whitespace | Runs of whitespace | `text` | `:136-147` |

Important scanner behaviours:

- **An unclosed `<` is emitted literally.** The scanner precomputes
  `gt_remaining[i]`, the count of `>` at or after char offset `i`
  (`:44-50`), and only enters variable mode when a closing `>` exists
  (`:84-87`). The rest of the line keeps its normal token styling — see the
  test `test_unclosed_variable_keeps_styling_rest_of_line` (`:206-216`).
- **A backslash-escaped `<` or `>` is not a variable.** The escape branch
  consumes the `\` and emits the bare `<` / `>` in the default colour
  (`:65-72`) — the backslash itself is dropped from the rendered text.
- **A trailing lone backslash is preserved** as an escape span rather than
  being dropped (`:167-171`).
- **A comment must be at a token boundary** — the previous span must be all
  whitespace (`:102-107`), so `foo#bar` is not a comment.
- **A `#` consumes the rest of the line**, including any quotes inside it
  (`:108-112`).

### The shell-keyword list

`src/utils/shell_keywords.rs` holds `SHELL_KEYWORDS: &[&str]` — **190**
entries — and a `LazyLock<HashSet<&'static str>>` view
(`SHELL_KEYWORDS_SET`, `:197-198`) for O(1) membership tests. The list spans
version control (`git`, `svn`, `hg`), containers (`docker`, `kubectl`, `helm`,
`podman`, `nerdctl`), JS/package managers (`npm`, `npx`, `pnpm`, `yarn`,
`node`, `bun`), Rust (`cargo`, `rustc`, `rustup`), build systems (`make`,
`cmake`, `meson`, `ninja`), cloud CLIs (`aws`, `gcloud`, `az`, `terraform`,
`terragrunt`), package managers (`apt`, `apt-get`, `yum`, `dnf`, `pacman`,
`zypper`, `brew`, `pip`, `pip3`, `poetry`), editors (`vim`, `vi`, `nano`,
`emacs`, `code`, `subl`, `helix`), privilege/user tools (`sudo`, `su`,
`chroot`, `useradd`, `usermod`, `userdel`, `groupadd`, `passwd`), and
session/system tools (`screen`, `tmux`, `tmuxp`, `loginctl`, `cron`, `at`,
`crontab`, `date`, `uptime`, `whoami`, …).

Membership is by **exact whole-word match** (`shell_keywords.contains(word.as_str())`,
`src/ui/highlight.rs:161`). A word is `[A-Za-z0-9_.-]+` (`:152`), so
`git.exe` or `docker-compose` match as single words — `docker-compose` is not
in the list and therefore renders as plain text.

### Variable highlighting vs. variable parsing

The `<…>` token the highlighter colours is scanned **independently** of
`src/utils/variables.rs`. The highlighter only cares that a `<` has a
matching `>`; it does not know about `=default` or the
`<name=|_a_||_b_||>` choice syntax. `src/utils/variables.rs` owns the real
grammar:

```rust
// src/utils/variables.rs:33-43
pub enum VariableKind {
    Required,
    DefaultValue(String),
    Choices { values: Vec<String>, default_index: Option<usize> },
}
```

The two scanners agreeing on `<…>` is a **convention, not a shared
implementation**. `src/utils/variables.rs` contains no colour or style code
at all — all highlighting lives in `src/ui/highlight.rs`.

### Where highlighting is invoked

The highlighter is **lazy and budgeted**, not precomputed at startup:

- `highlighted_commands: Vec<Option<Line<'static>>>` is allocated once
  (`src/ui/mod.rs:688`) and filled on demand.
- `command_line_for_display` (`src/ui/mod.rs:444-458`) returns the cached
  line, or falls back to unstyled text when the cache is cold.
- `warm_visible_highlights` (`src/ui/mod.rs:460-486`) fills the cache for the
  visible rows only, under a **2 ms per-frame budget**
  (`HIGHLIGHT_WARM_BUDGET`, `src/ui/mod.rs:749`), and only when no input is
  pending (`:1271-1289`).
- A theme change invalidates the whole cache (`pending_rehighlight` →
  `fill_with(|| None)`, `src/ui/mod.rs:828-833`).

## Key Files

| File | Subject |
|------|---------|
| `src/ui/theme.rs` (1063 lines) | `Theme` palette, Halloy TOML schema + projection, `ThemeManager`, `themes.toml`, initial resolution order, name validation |
| `src/ui/highlight.rs` (248 lines) | `highlight_command` — the shell tokenizer and its six highlighted token classes |
| `src/ui/_generated_bundled_themes.rs` (301 lines) | **Generated** — `DEFAULT_BUNDLED`, `BUNDLED`, `bundled_themes_decoded` |
| `src/utils/shell_keywords.rs` (198 lines) | `SHELL_KEYWORDS` (190 names) + `SHELL_KEYWORDS_SET` |
| `src/utils/variables.rs` (1947 lines) | Variable grammar — `parse_variables`, `expand_command`, `strip_escape_sequences` |
| `scripts/build_themes.py` | The only legitimate writer of the generated themes file |
| `themes/*.toml` (50 files) | Halloy theme sources, including `Cyber Red.toml` (the embedded default) |
| `src/ui/mod.rs:529-612` | Theme-picker plumbing: `ensure_theme_picker_ready`, preview, commit, cancel |
| `src/ui/theme.rs:639` | `ACTIVE_THEME` — the process-global `RwLock<Theme>` |
| `src/ui/mod.rs:460-486` | `warm_visible_highlights` — the 2 ms per-frame highlighting budget |

## Invariants

1. **`src/ui/_generated_bundled_themes.rs` is generated and must never be
   hand-edited.** Its only writer is `python3 scripts/build_themes.py`.
   Cargo does not run the script.
2. **`Theme` has exactly 10 non-optional colour fields.** Every projection
   path ends in `Color::White`, never `Color::Reset`
   (`src/ui/theme.rs:373-380`).
3. **The active theme is a process-global `RwLock<Theme>`** published through
   `get_theme()` / `set_active_theme()` (`src/ui/theme.rs:639-653`). The draw
   loop must read it once per frame, not per span.
4. **Theme resolution always terminates in a usable palette.** Missing file,
   corrupt `themes.toml`, unparsable theme, and absent `SNP_THEME` all fall
   through to `DEFAULT_BUNDLED` and then `DARK_THEME`.
5. **`themes.toml` stores a basename, not a path**, and every code path that
   turns a name into a path validates it first (`validate_theme_name`,
   `:605-633`).
6. **Bundled theme extraction is idempotent and non-destructive.**
   `write_theme_if_missing` never overwrites an existing file.
7. **Highlighting is lazy and budgeted**, so a large library does not pay a
   full-library syntax-highlighting cost on the first frame.
8. **The highlighter never panics on malformed input.** An unclosed `<`
   degrades to literal text; a trailing backslash is preserved.
9. **Theme names are not sanitized into a fixed set** — spaces, parentheses,
   dots, dashes, and underscores are legal, because the on-disk community
   themes use them.

## Gotchas

- **Do not edit the generated file.** Regenerate with
  `python3 scripts/build_themes.py` after changing anything under `themes/`.
  The generated header says so at `src/ui/_generated_bundled_themes.rs:1-3`.
- **`themes/` is seeded lazily**, on the first theme-picker open
  (`src/ui/mod.rs:540`) — not on first launch. A fresh install has no
  `themes/` directory until then.
- **`themes.toml` and `themes/*.toml` are different files.** The first is the
  selection index (`active = "…"`), the second are the definitions.
- **`themes.toml` is written with `Durability::SensitiveConfig`**
  (`src/ui/theme.rs:563-567`), so it is `0o600` on Unix and symlink writes
  are rejected — stricter than a theme index needs, but it is the current
  behaviour.
- **`SNP_THEME` is overloaded.** `dark`/`bright`/`light`/`auto` are legacy
  built-ins; *any other value* is interpreted as a filename in `themes/`
  (`src/ui/theme.rs:697-728`). A typo therefore silently falls through to
  the bundled default rather than erroring.
- **`SNP_THEME=auto` reads `COLORFGBG`** and treats a `15;` or `7;` prefix
  as a light terminal (`:701-706`).
- **The highlighter's keyword list is exact-match only.** `docker-compose`
  and `git.exe` are not in the 190-name list, so they render as plain text.
- **A `#` inside a token is not a comment.** The comment rule requires the
  previous span to be whitespace-only (`src/ui/highlight.rs:102-107`).
- **A `#` comment swallows the rest of the line**, quotes included.
- **A backslash-escaped `<` or `>` is rendered without the backslash.**
  `src/ui/highlight.rs:65-72` consumes the `\` and emits only `<` / `>` with
  the default colour, so the concatenated span text is **not** always equal
  to the input command. `\<` renders as `<`. The dropped backslash is the
  one the shell escape semantics intend (`strip_escape_sequences` performs
  the same removal for execution), but do not treat the highlight output as
  byte-for-byte lossless.
- **The highlighter and the variable parser are separate scanners.** A
  `<name=default>` token is coloured as one variable span, but the
  highlighter never inspects the `=default` part.
- **`src/ui/variables.rs` and `src/utils/variables.rs` have overlapping
  names.** The former is the modal prompt UI; the latter is the grammar and
  expander. `src/utils/mod.rs:20-23` re-exports only the latter's helpers.
- **`get_theme()` takes a read lock on every call.** The draw loop snapshots
  it once per frame (`src/ui/mod.rs:977`); do not call it per span.
