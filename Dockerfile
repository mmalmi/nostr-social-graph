ARG RUST_PACKAGE=nostr-social-graph-server
ARG RUST_BINARY=nostr-social-graph-server

FROM rust:1-bookworm AS builder

ARG RUST_PACKAGE
ARG RUST_BINARY

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY rust/Cargo.toml rust/Cargo.lock rust/
COPY rust/crates rust/crates

RUN cargo build --locked --manifest-path rust/Cargo.toml -p ${RUST_PACKAGE} --bin ${RUST_BINARY} --release

FROM debian:bookworm-slim

ARG RUST_BINARY

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/rust/target/release/${RUST_BINARY} /usr/local/bin/nostr-social-graph-app

ENV PORT=3000
ENV DATA_DIR=/app/data
ENV RUST_LOG=info
ENV APP_BINARY=${RUST_BINARY}

EXPOSE 3000

CMD ["/bin/sh", "-lc", "exec /usr/local/bin/nostr-social-graph-app"]
