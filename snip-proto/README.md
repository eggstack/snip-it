# snip-proto

gRPC protocol definitions for [snip-it](https://github.com/eggstack/snip-it) sync.

This crate contains the generated protobuf types and tonic client/server stubs used by the `snp` CLI and the `snip-sync` server for cross-device snippet synchronization.

## Usage

```toml
[dependencies]
snip-proto = "0.1"
```

The generated code is committed to the repository, so `protoc` is not required for normal builds. You only need the protobuf compiler if you regenerate the stubs from `proto/sync.proto`.

## License

[MIT](https://github.com/eggstack/snip-it/blob/main/LICENSE) © David Bowman
