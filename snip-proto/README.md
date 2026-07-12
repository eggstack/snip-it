# snip-proto

gRPC protocol definitions for [snip-it](https://github.com/eggstack/snip-it) sync.

This crate contains the generated protobuf types and tonic client/server stubs used by the `snp` CLI and the `snip-sync` server for cross-device snippet synchronization.

## Usage

```toml
[dependencies]
snip-proto = "0.1"
```

The generated code is committed to the repository, so `protoc` is not required for normal builds. You only need the protobuf compiler if you regenerate the stubs from `proto/sync.proto`.

## Wire contract

All authenticated RPCs accept the API key in the `authorization` metadata as
`Bearer <api-key>`. The `api_key` request fields remain for compatibility with
older clients, but new clients should leave them empty so credentials are not
duplicated in the protobuf body. `Register` and `Health` are unauthenticated.

Snippet synchronization uses Unix-second `updated_at` timestamps and
last-write-wins conflict resolution. A sync request returns records newer than
`last_sync_timestamp`; pagination uses `limit`, `offset`, `has_more`, and
`total_count`. Deleted snippets are represented as tombstones and are included
by `Sync` so clients can remove their local copies.

The `encrypted` flag indicates that the client has encrypted the description,
command, and tags into the `command` field. The server treats that payload as
opaque and stores it without decrypting it.

## License

[MIT](https://github.com/eggstack/snip-it/blob/main/LICENSE) © David Bowman
