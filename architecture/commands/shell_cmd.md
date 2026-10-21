# shell_cmd — Shell Integration Code Generation

[← Back to Overview](../overview.md)

## Overview

`src/commands/shell_cmd.rs` (1495 lines, mostly generated-script
templates plus unit tests) prints shell functions that wire `snp` into
interactive use: TUI selection into the command line, capture of the
current buffer into `snp new`, and previous-history helpers.

It installs **nothing**. No keybinding is bound, no rc file is edited,
no shell state is changed. The output is printed for inspection before
the user sources it. `snp doctor --check-shell` syntax-validates the
same generated output.

Related: [keybindings_cmd.md](keybindings_cmd.md) (the TUI key table),
[../ui.md](../ui.md), [../diagnostics.md](../diagnostics.md) (the doctor
check).

## CLI surface

`snp shell init <bash|zsh|fish>` (alias `i`) — `src/main.rs:185-189` and
`src/main.rs:254-262`; dispatched at `src/main.rs:807-809`.

```rust
// src/commands/shell_cmd.rs:7-15
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ShellIntegration {
    Bash,
    Zsh,
    Fish,
}
```

`ShellIntegration` is the **canonical CLI spelling** and is shared with
`snp doctor --check-shell` (`src/commands/doctor_cmd.rs:34`) so the shell
names cannot drift between generation and diagnostics.

- `as_str()` (`:19-25`) — the lowercase name used by doctor diagnostics.
- `to_shell_type()` (`:28-34`) — maps to the code-generation enum.

```rust
// src/commands/shell_cmd.rs:38-53
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellType {
    Bash,
    Zsh,
    Fish,
}
```

`run(shell: ShellType)` (`:56-64`) dispatches to `generate_bash` (`:68`),
`generate_zsh` (`:201`), or `generate_fish` (`:333`) and prints the
template verbatim with `print!` — the template owns its own bytes, so no
trailing newline is added.

`ShellIntegration` (CLI spelling) and `ShellType` (codegen) are
deliberately separate enums — do not merge them.

## Generated functions

| Shell | Public functions | Internal helpers |
|-------|------------------|------------------|
| Bash | `snp_select_raw` (`:125`), `snp_select_expanded` (`:129`), `snp_new_current` (`:133`), `snp_new_previous` (`:152`) | `__snp_select` (`:74`) |
| Zsh | `snp_select_raw` (`:263`, `zle -N`), `snp_select_expanded` (`:264`, `zle -N`), `snp_new_current` (`:284`), `snp_new_previous` (`:320`) | `__snp_select` (`:208`), `__snp_new_current` (`:266`), `__snp_new_previous` (`:288`) |
| Fish | `snp_select_raw` (`:388`), `snp_select_expanded` (`:392`), `snp_new_current` (`:402`), `snp_new_previous` (`:418`) | `__snp_select` (`:340`), `__snp_current_buffer` (`:398`) |

The four public names are identical across all three shells, so a user
can bind the same key to `snp_select_raw` in bash, zsh, and fish.

Each template ends with commented-out binding instructions:

- bash (`:192-196`): `bind -x '"\C-o": snp_select_raw'` and friends, to be
  added to `.bashrc`;
- zsh (`:324-328`): `bindkey '^O' snp_select_raw` and friends, to be
  added to `.zshrc`;
- fish (`:449-453`): `bind \co snp_select_raw` and friends, to be added
  to `config.fish`.

## Flow / steps

### Selection adapters (all three shells)

1. Guard: `command -v snp` (bash/zsh) or `command -q snp` (fish); on
   failure print `snp: 'snp' not found in PATH` to stderr and `return 1`.
2. Save the buffer: `$READLINE_LINE` / `$READLINE_POINT` (bash),
   `$BUFFER` / `$CURSOR` (zsh), `commandline` / `commandline -C` (fish).
3. Create a temp file with `mktemp` and pass it to
   `snp select --output-file <tmp> --raw|--expanded`, adding
   `--query <buffer>` when the buffer is non-empty.
4. **Exit code 4 (cancelled)** → delete the temp file, restore the
   saved buffer, and return 0 (bash/zsh) or `return` (fish). This is
   load-bearing: a cancelled selection must leave the line untouched.
5. Any other non-zero exit → delete the temp file, restore the buffer,
   and propagate the exit code.
6. Missing output file → print
   `snp: selection output was not produced`, restore the buffer, `return 1`.
7. Read the file with `IFS= read -r -d ''` (bash/zsh) and assign it to
   the buffer, setting the cursor to the end of the string.

The selection adapters use a **temp file** because stdout would add or
mangle newlines. `snp select --output-file` writes the file via an
atomic temp-file rename, so a pre-existing caller-owned file survives a
cancelled selection and a racing symlink cannot redirect the write
(`src/commands/select_cmd.rs:61-68`).

### Creation adapters

Creation adapters pass command text over **stdin** — never `eval`:

- bash `snp_new_current`: `printf '%s' "$old_line" | command snp new --command-stdin "$@"`
  (`:145`), restoring the buffer afterwards and propagating
  `${PIPESTATUS[1]}`.
- bash `snp_new_previous`: `fc -ln -2 -2` (or `-1` when invoked as a
  `bind -x` widget) into a temp file, then `tail -c +3` to strip exactly
  the two formatter bytes `fc` prepends, then
  `command snp new --command-stdin "$@" <"$tmp_file"` (`:152-190`). The
  two-byte strip keeps Bash 3.2 and Bash 4+ output identical while
  preserving legitimate leading whitespace in continuation lines.
- zsh `snp_new_current` / `snp_new_previous`: the same protocol against
  `$BUFFER` / `$CURSOR` (`:266-322`), propagating `${pipestatus[2]}`.
- fish `snp_new_current`: `printf '%s' "$current" | command snp new --command-stdin $argv`
  (`:411`), restoring `commandline` and the cursor afterwards.
- fish `snp_new_previous`: `history search --max=2 --null` — two entries,
  because a normal function invocation is itself the newest history event
  while a widget sees the latest accepted command as item one
  (`:424-439`).

Fish's `__snp_current_buffer` (`:398-400`) strips exactly the single
newline terminator `commandline` adds, so a buffer-supplied trailing
newline is preserved as data.

Every helper restores the prior buffer on any failure path.

## The shell-syntax validation used by `snp doctor`

`snp doctor --check-shell <shell>` is implemented by `check_shell_init`
(`src/commands/doctor_cmd.rs:1084-1225`):

1. Map the shell name to `ShellType`; an unknown name produces a
   `compat.shell_init.<shell>.unknown` **Warning** diagnostic suggesting
   bash, zsh, or fish (`:1089-1101`).
2. Generate the init code with the same `shell_cmd::generate_*` functions
   the CLI uses (`:1104-1108`) — the doctor and the CLI can never
   disagree about the text.
3. Check that the shell is on `PATH` with `which <shell>`. If it is not,
   emit `compat.shell_init.<shell>.unavailable` — **Error** in `--strict`
   mode, **Warning** otherwise — and stop (`:1110-1137`).
4. Write the code to a `NamedTempFile` (`:1140-1166`; a temp-file failure
   is a Warning diagnostic).
5. Run the shell's syntax checker (`:1169-1182`):

   | Shell | Command |
   |-------|---------|
   | bash | `bash -n <file>` |
   | zsh | `zsh -n <file>` |
   | fish | `fish --no-execute <file>` |

6. On success, emit `compat.shell_init.<shell>.ok` with severity **Info**
   and the byte count of the generated code (`:1185-1193`); on failure,
   a diagnostic carrying the shell's stderr.

## Mutation vs read-only

Read-only codegen: the command prints to stdout and writes no files, no
shell configuration, and no keybindings.

The *generated* helpers invoke `snp select` and `snp new`, which have
their own contracts — `snp select` is read-only, `snp new` creates a
snippet and notifies `SnippetCreate/User`.

## Tokio runtime

None. `snp shell init` never touches the global `RUNTIME`
(`src/main.rs:29-35`).

Startup policy: `Commands::Shell { .. }` uses
`StartupRecoveryPolicy::SuppressConfiguration` with
`StartupServices::Logging` (`src/main.rs:1019-1029`).

## Auto-sync trigger

None at generation time. At *use* time the helpers inherit the underlying
commands' behavior: `select` never notifies; `new --command-stdin`
notifies `SnippetCreate/User`.

## Error / exit mapping

`SnipResult<()>` — generation is infallible in practice (a typed
`ShellType` always maps to a template). An unknown shell spelling is
rejected by clap before `run` is reached (exit 2).

Helper-level failures — `snp` not on `PATH`, a cancelled selection, a
missing output file — are `return 1` **inside the generated shell code**,
not CLI errors. The generated helpers are never executed by `snp`, so
they cannot fail the `snp shell init` process.

## Key Files

| Path | Role |
|------|------|
| `src/commands/shell_cmd.rs` | `ShellIntegration`, `ShellType`, the three generators, `run` |
| `src/main.rs:185-189`, `:228-236`, `:770-772` | Clap wiring and dispatch |
| `src/commands/doctor_cmd.rs:1084-1225` | `check_shell_init` — the syntax validation |
| `src/commands/select_cmd.rs:61-123` | `snp select --output-file` — the lossless transport contract |
| `src/commands/new_cmd.rs` | `snp new --command-stdin` — the stdin creation contract |

## Invariants

- **Never auto-install.** No keybindings, no rc-file edits — the user
  inspects and sources explicitly.
- **Lossless transport.** Temp file + `read -d ''` for selection (stdout
  would add or mangle newlines); stdin pipe for creation (never `eval`).
- **Exit code 4 (cancelled) is load-bearing.** Only that code is treated as
  a clean "user cancelled" outcome (the helper restores the buffer and
  returns 0); every other non-zero code restores the buffer *and*
  propagates the code. Keep `select_cmd`'s `Cancelled → 4` mapping stable.
- **The four public function names are identical across bash, zsh, and
  fish.**
- **The doctor validates the same bytes the CLI prints** — both call
  `shell_cmd::generate_*`.

## Gotchas

- **`snp shell init` does not bind anything.** The templates print the
  `bind` / `bindkey` lines as comments.
- **The generated code is not executed by `snp`** — helper failures
  surface as shell-level `return 1`.
- **Zsh requires ZLE.** The zsh widgets are registered with `zle -N` and
  only work inside a ZLE widget context.
- **Fish's `snp_new_previous` needs two history entries**, because the
  function call itself is a history event.
- **`ShellIntegration` and `ShellType` are distinct enums** — do not
  merge them.
