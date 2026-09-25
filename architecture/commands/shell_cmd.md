# shell_cmd — Shell Integration Code Generation

[← Back to Overview](../overview.md)

## Overview

`src/commands/shell_cmd.rs` (1495 lines, mostly generated-script
templates) prints shell functions that wire `snp` into interactive
use: TUI selection into the command line, buffer capture into
`snp new`, and history helpers. It installs no keybindings and mutates
no shell config — output is printed for inspection before sourcing.
`snp doctor --check-shell` syntax-validates this same output.

## CLI surface

`snp shell init <bash|zsh|fish>` (alias `i`), `main.rs:228-236,769-773`.
`ShellIntegration{Bash,Zsh,Fish}` (`shell_cmd.rs:8`, `ValueEnum`) is
the CLI spelling; `as_str()` gives the lowercase name for doctor and
`to_shell_type()` maps to `ShellType{Bash,Zsh,Fish}` (`:39`,
`Display`). `run(shell: ShellType)` (`:56`) dispatches to
`generate_bash` (`:68`) / `generate_zsh` (`:201`) /
`generate_fish` (`:333`); output goes through `print!` (no trailing
newline added — the template owns its bytes).

## Flow / steps

1. `run` selects the generator and prints the script verbatim.
2. Bash (`__snp_select`, `snp_select_raw/_expanded`,
   `snp_new_current`, history helpers): selection via
   `mktemp` + `snp select --output-file $tmp --raw|--expanded
   [--query $READLINE_LINE]`; exit 4 (cancelled) restores the buffer;
   creation via `printf %s $READLINE_LINE | snp new --command-stdin`.
3. Zsh: same protocol against `BUFFER`/`CURSOR` (widgets, no
   Readline dependency); fish: same against `commandline`.
4. Contract details: selection adapters pass the temp file
   (`--output-file`) for lossless transport and read it with
   `read -r -d ''`; creation adapters pipe text over stdin with no
   shell evaluation; every helper guards `command -v snp` first and
   restores the prior buffer on any failure path.

The canonical `ShellIntegration` enum is shared with
`doctor --check-shell` so shell spellings cannot drift between
generation and diagnostics.

## Mutation vs read-only

Read-only codegen: prints to stdout, writes no files, changes no
shell state. (The *generated* helpers invoke `snp select`/`snp new`,
which have their own read-only/mutating contracts — see
`select_cmd.md`, `new_cmd.md`.)

## Auto-sync trigger

None at generation time. At *use* time, the helpers inherit the
underlying commands' behavior: `select` never notifies;
`new --command-stdin` notifies `SnippetCreate/User`.

## Error / exit mapping

`SnipResult<()>`; generation is infallible in practice (template
lookup cannot fail for a typed `ShellType`). Unknown shell spellings
are rejected by clap before `run`. Helper-level failures (missing
`snp`, cancelled selection, empty output file) are shell `return 1`
inside the generated code, not CLI errors.

## Key invariants

- Never auto-install: no keybindings, no rc-file edits — the user
  inspects and sources explicitly.
- Lossless transport: temp-file + `read -d ''` for selection (stdout
  would add/mangle newlines); stdin pipe for creation (never `eval`).
- Exit 4 (cancelled) is load-bearing: helpers restore the buffer only
  on this code; keep `select_cmd`'s `Cancelled → 4` mapping stable.
- `Shell` (completions) vs `ShellIntegration` (init) are distinct
  enums — do not merge them.

## File / line references

- Enums/run: `src/commands/shell_cmd.rs:8,39,56`
- Generators: `:68 (bash), :201 (zsh), :333 (fish)`
- Consumer contract: `src/commands/select_cmd.rs:69`
- Syntax check: `src/commands/doctor_cmd.rs:1084`
- Dispatch: `src/main.rs:228-236,769-773`
