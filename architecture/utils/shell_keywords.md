# shell_keywords.rs — Shell Command Names for Syntax Highlighting

## Overview

Provides a static list of ~190 shell command names used by the TUI syntax highlighter to colorize the first token of snippet commands.

**File**: `src/utils/shell_keywords.rs`

## Data

```rust
pub const SHELL_KEYWORDS: &[&str] = &[...];
```

Includes common tools across categories:
- **Version control**: git, svn, hg
- **Containers**: docker, kubectl, helm, podman, nerdctl
- **Package managers**: npm, npx, pnpm, yarn, node, bun, cargo, rustc, rustup
- **Build tools**: make, cmake, meson, ninja
- **Cloud**: aws, gcloud, az, terraform, terragrunt
- **Shell builtins**: ls, cd, pwd, mkdir, rm, cp, mv, cat, echo, etc.
- **Networking**: curl, wget, ssh, scp, rsync
- **Text processing**: grep, sed, awk, jq, sort, uniq, wc
- **System**: ps, top, kill, chmod, chown, tar, zip, unzip

## Usage

Used by `src/ui/highlight.rs` to identify the command token for coloring in the TUI. The highlighter checks if the first word of a snippet command matches a keyword to apply the accent color.
