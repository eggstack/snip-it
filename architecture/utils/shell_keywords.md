# shell_keywords.rs — Shell Command Names for Syntax Highlighting

[← Back to Overview](../overview.md)

## Overview

Provides a static list of shell command names used by the TUI syntax highlighter to colorize command tokens in snippet commands.

**File**: `src/utils/shell_keywords.rs` (198 lines, no tests — pure data)

## Data

```rust
pub const SHELL_KEYWORDS: &[&str] = &[...]; // 190 entries

pub static SHELL_KEYWORDS_SET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| SHELL_KEYWORDS.iter().copied().collect());
```

The slice is the canonical ordered list; the `LazyLock<HashSet>` is built once on first use for O(1) membership checks. Both hold `&'static str` — no allocation beyond the set itself.

## Categories (190 keywords)

- **Version control** (3): git, svn, hg
- **Containers** (5): docker, kubectl, helm, podman, nerdctl
- **JS runtimes / package managers** (7): npm, npx, pnpm, yarn, node, bun + pip/pip3/poetry below
- **Rust toolchain** (3): cargo, rustc, rustup
- **Build tools** (4): make, cmake, meson, ninja
- **Cloud / IaC** (5): aws, gcloud, az, terraform, terragrunt
- **Coreutils / shell builtins** (~50): ls, cd, pwd, mkdir, rm, cp, mv, cat, echo, printf, export, source, env, set, unset, read, exit, trap, exec, eval, test, time, timeout, …
- **Text processing** (~15): grep, egrep, fgrep, sed, awk, find, xargs, sort, uniq, head, tail, wc, cut, tr, tee
- **Networking** (~15): curl, wget, ssh, scp, rsync, netstat, ss, ip, ifconfig, ping, traceroute, nslookup, dig, host, nmap, telnet, ftp, sftp
- **Archives** (8): tar, zip, unzip, gzip, gunzip, bzip2, xz, 7z
- **Process / system** (~25): systemctl, journalctl, ps, kill, killall, pkill, pgrep, top, htop, btop, df, du, mount, chmod, chown, stat, lsof, strace, gdb, …
- **Package managers (OS)** (7): apt, apt-get, yum, dnf, pacman, zypper, brew
- **Editors** (6): vim, vi, nano, emacs, code, subl, helix
- **Privilege / session** (~10): sudo, su, chroot, cron, crontab, screen, tmux, loginctl, passwd, …
- **Checksums / encoding** (6): md5sum, sha256sum, base64, xxd, od, hexdump
- **Misc** (date, cal, bc, factor, shuf, seq, watch, file, which, …)

## Usage

Used by `src/ui/highlight.rs` to identify command tokens for coloring in the TUI. The highlighter tokenizes each snippet command on whitespace/pipes and checks membership in `SHELL_KEYWORDS_SET`:

- Match → keyword color (command position).
- No match → default/argument color; flags (`--foo`, `-x`) get flag coloring regardless.

Because lookup is a `HashSet<&'static str>` probe, highlighting cost is linear in token count with no per-keyword scan.

## Lookup Semantics

- Exact, case-sensitive match on the full token (`Git` and `git.exe` do not match).
- Only the command position is semantically a "keyword", but the set probe is applied per token — arguments that happen to equal a tool name (e.g. `echo git`) still colorize. This is accepted: highlighting is cosmetic, never semantic.
- Empty commands and pure-flag lines short-circuit before lookup.

## Maintenance

Adding a keyword is a one-line append to `SHELL_KEYWORDS`, grouped by category; `SHELL_KEYWORDS_SET` picks it up automatically with no other change. When adding, prefer the bare binary name (`kubectl`, not `kubectl.exe`); platform-specific suffixes are out of scope.

## Invariants

- Data-only module: no logic, no I/O, no platform gates — safe to use from any layer.
- Adding a keyword is a one-line append; ordering is cosmetic (grouped by category).
- `SHELL_KEYWORDS_SET` must stay derived from `SHELL_KEYWORDS` — never a separate hand-maintained list.
