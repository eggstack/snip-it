FROM rust:1.81 as builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-references libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/snip-sync /app/

ENV DATABASE_URL=/data/snippets.db
ENV GRPC_HOST=0.0.0.0
ENV GRPC_PORT=50051
ENV HTTP_HOST=0.0.0.0
ENV HTTP_PORT=50050

VOLUME /data

EXPOSE 50051 50050

ENTRYPOINT ["/app/snip-sync"]
