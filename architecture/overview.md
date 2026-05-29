# Architecture Overview

Bird's eye view of the snip-it codebase — a terminal-based snippet manager with fuzzy search, clipboard support, variable expansion, TUI interface, and cloud sync with end-to-end encryption.

## System Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                         snp (CLI Client)                           │
│                                                                     │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────────────┐   │
│  │ Commands │  │   TUI    │  │ Clipboard│  │   Variables      │   │
│  │ (13 cmds)│  │ (ratatui)│  │ (copypasta│  │ (expand/prompt) │   │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └───────┬──────────┘   │
│       │              │              │                │               │
│  ┌────┴──────────────┴──────────────┴────────────────┴───────────┐  │
│  │                    Core Modules                                │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────────┐  │  │
│  │  │ Library  │ │  Config  │ │ Encryption│ │   Error Types    │  │  │
│  │  │ (TOML)   │ │ (sync)   │ │ (AES-GCM)│ │   (SnipError)    │  │  │
│  │  └──────────┘ └──────────┘ └──────────┘ └──────────────────┘  │  │
│  └────────────────────────────┬──────────────────────────────────┘  │
│                               │                                     │
│  ┌────────────────────────────┴──────────────────────────────────┐  │
│  │                    Sync Layer                                  │  │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────────────┐  │  │
│  │  │   Sync   │ │   Sync   │ │   Sync   │ │   Logging &      │  │  │
│  │  │  Client  │ │Commands  │ │ Settings │ │   Audit Log      │  │  │
│  │  │ (gRPC)   │ │ (merge)  │ │ (TOML)   │ │   (tracing)      │  │  │
│  │  └────┬─────┘ └──────────┘ └──────────┘ └──────────────────┘  │  │
│  └───────┼────────────────────────────────────────────────────────┘  │
└──────────┼───────────────────────────────────────────────────────────┘
           │ TLS/gRPC
           ▼
┌──────────────────────────────────────────────────────────────────────┐
│                      snip-sync (Server)                              │
│                                                                      │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐  │
│  │  gRPC    │ │  HTTP/   │ │ Database │ │  Rate    │ │ Premade  │  │
│  │ Service  │ │  Axum    │ │ (SQLite) │ │ Limiter  │ │ Manager  │  │
│  └──────────┘ └──────────┘ └──────────┘ └──────────┘ └──────────┘  │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │                    snip-proto (Protobuf)                      │    │
│  │         Generated gRPC code from sync.proto                   │    │
│  └──────────────────────────────────────────────────────────────┘    │
└──────────────────────────────────────────────────────────────────────┘
```

## Workspace Layout

```
snip-it/
├── Cargo.toml              # Main crate: binary "snp" (Rust 1.81+)
├── src/                    # Client application source
│   ├── main.rs             # CLI entry, clap command dispatch
│   ├── commands/           # 13 CLI subcommands (one module each)
│   ├── utils/              # Shared utilities
│   ├── clipboard.rs        # Cross-platform clipboard
│   ├── config.rs           # Sync settings
│   ├── encryption.rs       # AES-256-GCM + Argon2id
│   ├── error.rs            # SnipError enum
│   ├── library.rs          # Snippet/Library data model
│   ├── logging.rs          # Tracing + audit log
│   ├── sync.rs             # gRPC client
│   ├── sync_commands.rs    # Sync orchestration + merge
│   └── ui/                  # TUI (ratatui) + fuzzy search
├── snip-proto/             # Protobuf definitions + generated code
│   ├── proto/sync.proto    # Service + message definitions
│   ├── build.rs            # tonic-build code generation
│   └── src/lib.rs          # Re-exports generated types
├── snip-sync/              # Server binary
│   ├── src/main.rs         # gRPC + HTTP server entry
│   ├── src/db.rs           # SQLite via sqlx
│   ├── src/rate_limiter.rs # Per-key rate limiting
│   ├── src/metrics.rs      # Prometheus counters
│   └── src/premade.rs      # Premade library file scanning
├── tests/
│   └── integration.rs      # CLI integration tests
└── architecture/           # This documentation
```

## Component Index

| Component | Location | Description |
|-----------|----------|-------------|
| [CLI Entry & Commands](cli.md) | `src/main.rs`, `src/commands/` | Clap-based CLI, 13 subcommands, command dispatch |
| [TUI Module](ui.md) | `src/ui/` | ratatui-based terminal UI, fuzzy search, themes, variable prompts |
| [Core Data Model](core.md) | `src/library.rs`, `src/error.rs` | Snippet, Snippets, LibraryManager, SnipError |
| [Configuration](config.md) | `src/config.rs`, `src/utils/config.rs` | SyncSettings, SyncDirection, config directory resolution |
| [Sync System](sync.md) | `src/sync.rs`, `src/sync_commands.rs` | gRPC client, merge logic, bidirectional sync |
| [Encryption](encryption.md) | `src/encryption.rs` | AES-256-GCM + Argon2id key derivation, end-to-end encryption |
| [Clipboard](clipboard.md) | `src/clipboard.rs` | Cross-platform clipboard, auto-clear scheduling |
| [Utilities](utils.md) | `src/utils/` | Variable expansion, TOML helpers, shell keywords, config paths |
| [Logging](logging.md) | `src/logging.rs` | Structured tracing, log rotation, panic handling, audit log |
| [Server](server.md) | `snip-sync/src/` | gRPC/HTTP server, SQLite database, rate limiting, metrics |
| [Protobuf API](proto.md) | `snip-proto/` | Service definitions, message types, generated gRPC code |

## Data Flow

### Snippet Lifecycle

1. **Create** (`snp new`) — User provides command, description, tags → stored in library TOML
2. **Browse** (`snp run/clip/search`) — TUI loads snippets, fuzzy filters, user selects
3. **Expand** — Variables `<name=default>` are parsed and prompted in TUI
4. **Execute** — Command runs via shell (`run`) or copies to clipboard (`clip`)
5. **Sync** (`snp sync`) — Local snippets encrypted, pushed to server, server snippets merged back
6. **Premade** (`snp premade`) — Browse/download community snippet libraries from server

### Sync Flow

```
Local                    Server
  │                        │
  ├── encrypt snippets ───►│
  │   (AES-256-GCM)        │
  │                        ├── upsert to SQLite
  │◄── return server ──────┤   (last-write-wins)
  │   snippets since       │
  │   last_sync_timestamp  │
  │                        │
  ├── decrypt server ──────┤
  │   snippets             │
  │                        │
  └── merge locally ───────┘
      (last-write-wins by updated_at)
      (local-only fields preserved)
```

## Key Design Decisions

- **TOML storage** — Snippets stored in human-readable TOML files, compatible with `pet` format
- **Library mode** — Multiple snippet libraries with primary designation, migrated from single-file
- **End-to-end encryption** — Server never sees plaintext snippet content (AES-256-GCM + Argon2id)
- **Last-write-wins merge** — Simple conflict resolution based on `updated_at` timestamp
- **Pre-computed TUI highlights** — Syntax highlighting computed once at startup, not in draw loop
- **Lazy async runtime** — Tokio runtime initialized only when async commands are invoked
- **Audit logging** — Every snippet execution/copy is logged with timestamps

## Deep Dive Navigation

Each component has its own detailed document in this `architecture/` directory. Start with the component you want to review in depth:

- **Quick review**: Start with [CLI Entry & Commands](cli.md) to understand user-facing behavior
- **Data model**: See [Core Data Model](core.md) for Snippet/Library structures
- **Sync focus**: Read [Sync System](sync.md) and [Server](server.md) together
- **Security review**: Check [Encryption](encryption.md) and [Server](server.md) for auth/crypto
- **UI review**: See [TUI Module](ui.md) for the interactive terminal interface
