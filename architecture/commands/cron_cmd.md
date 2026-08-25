# cron_cmd — Crontab Generation

## Overview

`cron_cmd` generates crontab entries for automatic periodic sync operations.

## Entry Point

```rust
pub fn run(interval: u32) -> SnipResult<()>
```

## Flow

1. Load sync settings from `~/.config/snp/sync.toml`
2. Determine sync interval
3. Generate crontab entry for the current user
4. Output to stdout or append to crontab

## Generated Crontab Entry

```cron
*/15 * * * * /path/to/snp sync
```
This runs sync every 15 minutes.

## Interval Mapping

| Interval Flag | Crontab |
|---------------|---------|
| `--interval 15` | `*/15 * * * *` |
| `--interval 60` | `*/60 * * * *` |
| `--interval 1` | `*/1 * * * *` |
| `--interval 0` | Error: "Interval must be at least 1 minute" |

## Flags

- `--interval <minutes>` — Sync interval in minutes (default: 15)

## Safety

- Prints the crontab entry to stdout for manual review
- Optionally copies to clipboard
- On Windows, prints Task Scheduler instructions instead

## Sync Mode

Generated entries use `snp sync` which respects the configured sync direction in `sync.toml`. The cron entry does not add extra flags — it relies on the user's saved sync configuration.

## Related

- [sync_cmd.md](sync_cmd.md) — Sync operation details
- [sync.md](../sync.md) — Sync settings and merge strategy
