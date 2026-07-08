FROM rust:1.94 AS builder

RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY snip-sync/Cargo.toml snip-sync/
COPY snip-proto/Cargo.toml snip-proto/
COPY snip-proto/build.rs snip-proto/
COPY snip-proto/proto snip-proto/proto
COPY snip-proto/src snip-proto/src
COPY snip-sync/src snip-sync/src
COPY snip-sync/config.toml snip-sync/config.toml

RUN cargo build --release -p snip-sync

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        libssl3 \
        ca-certificates \
        curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/snip-sync /app/

ENV DATABASE_URL=/data/snippets.db \
    GRPC_HOST=0.0.0.0 \
    GRPC_PORT=50051 \
    HTTP_HOST=0.0.0.0 \
    HTTP_PORT=50050 \
    SNIP_SYNC_ALLOW_HTTP=true

VOLUME /data

EXPOSE 50051 50050

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -fsS http://127.0.0.1:${HTTP_PORT}/health || exit 1

ENTRYPOINT ["/app/snip-sync"]
