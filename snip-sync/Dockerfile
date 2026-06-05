FROM rust:1.88 AS builder

WORKDIR /build

COPY snip-sync/Cargo.toml snip-sync/Cargo.lock snip-sync/
COPY snip-proto/Cargo.toml snip-proto/Cargo.lock snip-proto/
COPY snip-proto/build.rs snip-proto/
COPY snip-proto/proto snip-proto/proto
COPY snip-proto/src snip-proto/src
COPY snip-sync/src snip-sync/src

RUN cargo build --release -p snip-sync

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-references libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/snip-sync /app/

ENV DATABASE_URL=/data/snippets.db
ENV GRPC_HOST=0.0.0.0
ENV GRPC_PORT=50051
ENV HTTP_HOST=0.0.0.0
ENV HTTP_PORT=50050
ENV SNIP_SYNC_ALLOW_HTTP=true

VOLUME /data

EXPOSE 50051 50050

ENTRYPOINT ["/app/snip-sync"]
