# Pet file compatibility and migration

Snip-it can import Pet's TOML snippet format. The import command is the safest
way to move a collection because it creates a registered snip-it library and
reports fields or values that may need attention.

## Supported format

A basic Pet file looks like this:

```toml
[[snippets]]
description = "Show listening ports"
command = "lsof -iTCP -sTCP:LISTEN"
tag = ["network"]
output = ""
```

Snip-it accepts the common field spellings `description`/`Description`/`name`,
`command`/`Command`/`cmd`, `tag`/`tags`/`Tag`/`Tags`, and `output`/`Output`.
It also reads Pet's `[[Snippets]]` table spelling and handles common escaped
angle brackets in command strings. Variables such as `<host>` and
`<branch=main>` work in both tools.

Unknown Pet-specific fields are reported by diagnostics but are not retained
when imported. Imported snippets receive new IDs. Missing timestamps are
created, and sync-only state is reset. Keep a copy of the source file if it
contains fields or metadata you may need later.

## Differences to know

- Snip-it adds IDs, timestamps, folders, favorites, and sync state. Pet may
  ignore these fields, but editing and saving the same file with Pet can remove
  them. Use separate files if you plan to keep editing the collection in both
  tools.
- Pet stores expected or sample command output in `output` and displays it with
  the snippet. In snip-it, a non-empty `output` value is used by `snp run` as a
  stdout destination path. Review these values before running imported snippets.
- Pet and snip-it use different sync services. Snip-it sync requires the
  optional self-hosted `snip-sync` server. See the
  [server guide](../snip-sync/README.md).

## Import a collection

Start with a read-only report, then preview the import:

```bash
snp doctor --pet-file ~/.config/pet/snippets.toml
snp import pet ~/.config/pet/snippets.toml --dry-run
```

Import as a new library. If omitted, the library name is derived from the source
filename:

```bash
snp import pet ~/.config/pet/snippets.toml --library pet-snippets
```

To add snippets to an existing library, use `--merge`. Exact duplicates are
skipped. To replace an existing library, use `--replace`; snip-it backs up the
destination first. Add `--report json` for a machine-readable report or
`--strict` to treat warnings as errors. Run `snp import pet --help` for all
options.

The source file is not modified. After import, use `snp library list` to check
the new library and `snp list --library pet-snippets` to browse it.

## Shell-buffer workflow

Snip-it's optional shell integration can search using the current command
buffer, insert a selected command without running it, and restore the buffer if
you cancel. Generate it with `snp shell init bash`, `snp shell init zsh`, or
`snp shell init fish`. Inspect the output before sourcing. It installs no key
bindings by default. See [Shell integration in the User Guide](USER_GUIDE.md#shell-integration)
for setup instructions.

## Related guides

- [User Guide](USER_GUIDE.md)
- [Command Overview](COMMANDS.md)
- [Pet command reference](https://github.com/knqyf263/pet)
