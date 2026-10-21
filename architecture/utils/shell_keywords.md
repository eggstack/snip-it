# shell_keywords.rs — Shell Command Names for Syntax Highlighting

[← Back to Overview](../overview.md)

## Overview

`src/utils/shell_keywords.rs` is a **pure data module**: a list of shell
command names that the TUI's syntax highlighter uses to decide whether a word
in a snippet command should be painted as a keyword.

**File**: `src/utils/shell_keywords.rs` (198 lines — data only, **no tests**,
no `#[cfg(test)]` module).

**Not covered here:** the highlighter's tokenizer itself (which also
recognises flags, strings, variables, escapes, and comments — see
`src/ui/highlight.rs`), the theme colour definitions, and the snippet
variable grammar (see [utils/variables.md](variables.md)).

This module is intentionally **dumb data**. It has no behaviour of its own,
so its only contract is *what is in the list* and *who reads it*.

## The List

```rust
// src/utils/shell_keywords.rs:4-195
pub const SHELL_KEYWORDS: &[&str] = &[
    "git",
    "svn",
    "hg",
    "docker",
    "kubectl",
    ...
    "seq",
];
```

| Property | Value |
|----------|-------|
| Element count | **exactly 190** |
| Declaration | `src/utils/shell_keywords.rs:4` (`pub const SHELL_KEYWORDS: &[&str]`) |
| Entries | `src/utils/shell_keywords.rs:5` (`"git"`) … `src/utils/shell_keywords.rs:194` (`"seq"`) |
| Closing bracket | `src/utils/shell_keywords.rs:195` |
| Type | `&[&str]` — a borrowed slice of `&'static str` |
| Mutability | `const` — the list cannot be mutated at runtime |

The list is **not alphabetised**. It is **loosely grouped by category** —
version control first, then containers, language runtimes, build tools, cloud,
and a long tail of coreutils, text processing, networking, archives,
process/system tools, shell builtins, debuggers, checksums, OS package
managers, editors, privilege/session tools, and misc:

| Ordinals | Group | Examples |
|----------|-------|----------|
| 1–3 | Version control | `git`, `svn`, `hg` |
| 4–8 | Containers / orchestration | `docker`, `kubectl`, `helm`, `podman`, `nerdctl` |
| 9–14 | JS runtimes / package managers | `npm`, `npx`, `pnpm`, `yarn`, `node`, `bun` |
| 15–17 | Rust toolchain | `cargo`, `rustc`, `rustup` |
| 18–21 | Build tools | `make`, `cmake`, `meson`, `ninja` |
| 22–26 | Cloud / IaC | `aws`, `gcloud`, `az`, `terraform`, `terragrunt` |
| 27–37 | Core file / output commands | `ls`, `cd`, `pwd`, `mkdir`, `rm`, `cp`, `mv`, `cat`, `echo`, `printf` |
| 38–52 | Text processing | `grep`, `sed`, `awk`, `find`, `xargs`, `sort`, `uniq`, `head`, `tail`, `wc`, `cut`, `tr`, `tee` |
| 53–56 | Permission modes | `chmod`, `chown`, `chgrp`, `umask` |
| 57–74 | Networking | `curl`, `wget`, `ssh`, `scp`, `rsync`, `ping`, `dig`, `nmap` |
| 75–83 | Archives | `tar`, `zip`, `unzip`, `gzip`, `bzip2`, `xz`, `7z`, `rar` |
| 84–96 | Process / system | `systemctl`, `journalctl`, `ps`, `kill`, `pkill`, `top`, `htop` |
| 97–109 | Filesystem / disks | `df`, `du`, `ln`, `touch`, `file`, `fdisk`, `lsblk`, `mkfs`, `fsck` |
| 110–137 | Shell builtins / control flow | `which`, `alias`, `export`, `source`, `set`, `read`, `exit`, `trap`, `exec`, `eval`, `test`, `case`, `function`, `time`, `timeout`, `watch` |
| 138–142 | Debuggers / tracers | `strace`, `ltrace`, `gdb`, `lsof`, `stat` |
| 143–148 | Checksums / encoding | `md5sum`, `sha256sum`, `base64`, `xxd`, `od`, `hexdump` |
| 149–158 | OS package managers | `apt`, `apt-get`, `yum`, `dnf`, `pacman`, `zypper`, `brew`, `pip`, `pip3`, `poetry` |
| 159–165 | Editors | `vim`, `vi`, `nano`, `emacs`, `code`, `subl`, `helix` |
| 166–184 | Privilege / session / users | `sudo`, `su`, `chroot`, `cron`, `crontab`, `screen`, `tmux`, `loginctl`, `useradd`, `passwd`, `id`, `whoami` |
| 185–190 | Misc | `date`, `cal`, `bc`, `factor`, `shuf`, `seq` |

The grouping is a convenience for humans editing the list, not a contract —
the order carries no meaning to the highlighter.

Every entry is a **bare command name** — no path separators, no arguments, no
flags. The highlighter compares a whole word against the set, so a name with
a `/` in it would never match.

## The Hash Set

```rust
// src/utils/shell_keywords.rs:197-198
pub static SHELL_KEYWORDS_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| SHELL_KEYWORDS.iter().copied().collect());
```

`SHELL_KEYWORDS_SET` is a `std::sync::LazyLock<HashSet<&'static str>>`
derived from `SHELL_KEYWORDS` at first use.

It exists purely for **O(1) membership probes**. The highlighter tests one
word per token, so a linear scan of a 190-element slice inside the render
loop would be a measurable cost on every keystroke that re-renders the
preview. `LazyLock` (not `OnceLock` + a function) means the set is built
exactly once, lazily, on the first probe, with no explicit initialisation
call and no thread-safety ceremony at the call site.

Both symbols are `pub` and live in a `pub mod` inside the crate-private
`utils` module, so they are reachable as
`crate::utils::shell_keywords::SHELL_KEYWORDS_SET` from anywhere in the
`snip-it` crate and from nowhere else.

## The Only Consumer

`src/ui/highlight.rs` — the TUI syntax highlighter. There is exactly one
consumer:

```rust
// src/ui/highlight.rs:8
use crate::utils::shell_keywords::SHELL_KEYWORDS_SET;
```

| Step | Site |
|------|------|
| Dereference the `LazyLock` into a `&HashSet` | `src/ui/highlight.rs:62` — `let shell_keywords = &*SHELL_KEYWORDS_SET;` |
| Probe a word | `src/ui/highlight.rs:161` — `let is_kw = shell_keywords.contains(word.as_str());` |
| Apply the keyword colour | `src/ui/highlight.rs:162` — `color_keyword` (`theme.primary`) vs `color_default` (`theme.text`) |

The highlighter's word scanner accepts `alphanumeric`, `_`, `-`, and `.`
(`src/ui/highlight.rs:151-159`), so `git` matches, `git-lfs` matches only if
`git-lfs` is in the list, and `docker-compose` matches only if that exact
string is listed.

The set probe is applied to **every word token**, not only to the token in
command position — an argument that happens to equal a tool name (`echo git`)
is highlighted as a keyword too. This is accepted: highlighting is cosmetic
and never semantic.

There is **no short-circuit** for empty commands or flag-only lines; the
scanner simply runs and finds nothing to probe.

Nothing else reads the list. It is **not** used for command validation, for
shell parsing, for completion, or for the `snp` command surface — the
snippet command is never sanitized or interpreted against this list.

## Changing the List

Adding or removing a name is a **behavioural change to the TUI**, not a
data cleanup: every snippet containing that word changes colour in the
preview panel. There is no test in this module to update, and no test
elsewhere asserts the list's contents.

Practical consequences:

- Adding a name that is also a common English word (`let`, `if`, `do`,
  `test`, `time`, `date`) will paint ordinary prose in a keyword colour.
- Removing a name silently downgrades that word to default text.
- The count in this document (**190**) must be re-verified whenever the list
  changes:

```bash
awk 'NR>=5 && NR<=194' src/utils/shell_keywords.rs | grep -c '^\s*"'
```

## Key Files

| File | Role |
|------|------|
| `src/utils/shell_keywords.rs` | The 190-name list and the derived `HashSet` |
| `src/ui/highlight.rs:8`, `:62`, `:161` | The only consumer — the TUI syntax highlighter |

## Invariants

- **The list holds exactly 190 command names** — bare command names only.
- **`SHELL_KEYWORDS_SET` is derived, never hand-maintained.** It is built
  from `SHELL_KEYWORDS` at `src/utils/shell_keywords.rs:197-198`; editing the
  set directly is impossible by construction.
- **Membership is an exact whole-word match.** No prefix, suffix, or
  substring matching is performed.
- **The module is display-only.** It never gates execution, validation, or
  sanitization — snippet commands are stored and run verbatim by design.
- **Only the TUI highlighter reads it.** A grep for `SHELL_KEYWORDS_SET`
  outside `src/ui/highlight.rs` should return nothing.

## Gotchas

- **Adding a name changes rendering for every snippet that contains it.**
  There is no compile-time or test-time signal.
- **There are no tests in this module.** A change to the list is not covered
  by any assertion, in this file or elsewhere — the only guard is review.
- **`SHELL_KEYWORDS_SET` is a `LazyLock`, not a `static` array.** Deref it
  once (`&*SHELL_KEYWORDS_SET`) into a local, as the highlighter does at
  `src/ui/highlight.rs:62`, rather than re-derefing inside a loop.
- **The list is `const`, so the strings are `'static`.** Adding a runtime-
  computed name is not possible without changing the type.
- **The count in this doc (190) is verified against the current tree.** The
  entry range `src/utils/shell_keywords.rs:5-194` is derived from the
  current declaration; re-check it after any edit.
