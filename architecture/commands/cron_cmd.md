# cron_cmd — Crontab Generation

## Overview

`cron_cmd` generates crontab entries for automatic periodic sync operations.

## Entry Point

```rust
pub fn run(matches: &ArgMatches) -> SnipResult<()>
```

## Flow

1. Load sync settings from `~/.config/snp/sync.toml`
2. Determine sync interval
3. Generate crontab entry for the current user
4. Output to stdout or append to crontab

## Generated Crontab Entry

```cron
*/15 * * * * /path/to/snp sync --local
```
This runs sync every 15 minutes in local-only mode.

## Interval Mapping

| Interval Flag | Crontab |
|---------------|---------|
| `--interval 15` | `*/15 * * * *` |
| `--interval 60` | `0 * * * *` |
| `--interval 3600` | `0 */1 * * *` |
| `--interval 0` | Removes crontab entry |

## Flags

- `--install` — Append generated crontab to user's crontab
- `--remove` — Remove snip-it crontab entries
- `--interval <seconds>` — Sync interval override

## Safety

- `--install` appends only snip-it related entries
- Existing crontab entries preserved
- Uses `crontab -` to read/write safely

## Sync Mode

Generated entries use `--local` flag to avoid sync conflicts from multiple concurrent instances. For server sync, use a unique lock file mechanism.

## Related

- [sync_cmd.md](sync_cmd.md) — Sync operation details
- [sync.md](../sync.md) — Sync settings and merge strategy
