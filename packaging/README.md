# Bootstrap installers

The bootstrap installers are deployment helpers. They query each component's
crate metadata independently, download the matching release asset from its
exact GitHub tag, verify the SHA-256 sidecar, and verify `version` before
placing the executable. They fall back to an exact `cargo install --version
'=X.Y.Z' --locked` build only for source-only targets or a definite GitHub
asset 404. Checksum, transport, and candidate-version failures are hard
failures and never trigger Cargo fallback.

```bash
# snp (default)
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash

# snip-sync, or both independently versioned components
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash -s -- --server
curl -fsSL https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.sh | bash -s -- --both
```

For a pinned single component, add `--version X.Y.Z`. A single version is
rejected with `--both` because `snip-it` and `snip-sync` have independent
release tags and crate versions.

On Windows, download and inspect `install.ps1`, then run it in PowerShell:

```powershell
irm https://raw.githubusercontent.com/eggstack/snip-it/main/packaging/install.ps1 -OutFile .\install-snip-it.ps1
Get-Content .\install-snip-it.ps1
. .\install-snip-it.ps1 -Component Snp
```

The shorter `irm ... | iex` form is convenient but executes remote content
without an inspection step. User-local installs go under `$HOME/.local/bin` on
Unix and `$env:LOCALAPPDATA\snip-it` on Windows; the installer prints PATH
guidance when needed. Server installation initializes missing layout/config
only and delegates startup registration to `snip-sync startup install` when
that lifecycle command is available.
