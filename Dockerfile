FROM docker.io/library/rust:1.95-slim-bookworm AS builder

WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src

RUN cargo build --release --locked && strip target/release/aoe2-tournament-bot

FROM docker.io/library/debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/aoe2-tournament-bot /usr/local/bin/aoe2-tournament-bot

WORKDIR /app
# Mount /app/config.toml at runtime. CONFIG_PATH can override the path.
ENV RUST_LOG=info
CMD ["/usr/local/bin/aoe2-tournament-bot"]
