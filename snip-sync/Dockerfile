FROM rust:1.75 as builder

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/snip-sync /app/

ENV DATABASE_URL=/data/snippets.db
ENV SERVER_ADDR=0.0.0.0:3000

VOLUME /data

EXPOSE 3000

ENTRYPOINT ["/app/snip-sync"]
