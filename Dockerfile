# cargo-chef caches a dependency-only build as its own layer, so editing
# src/ no longer invalidates the (slow) dependency compile — only a
# Cargo.toml/Cargo.lock change does.
FROM docker.io/library/rust:1.95-slim-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /build

FROM chef AS planner
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Build only the dependencies from the recipe. This layer is cached until
# the recipe (i.e. Cargo.toml/Cargo.lock) changes. Flags must match the
# final `cargo build` below for the dependency cache to be reused.
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --release --locked --recipe-path recipe.json

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo build --release --locked && strip target/release/aoe2-tournament-bot

FROM docker.io/library/debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/aoe2-tournament-bot /usr/local/bin/aoe2-tournament-bot

WORKDIR /app
# Tournament routing is checked into git and baked into the image.
COPY tournaments.toml ./tournaments.toml
# Secrets/per-env config (config.toml) is provided at runtime — locally as a
# bind mount, in production from Secret Manager. The Worker Pool sets
# CONFIG_PATH to the mounted location; for local runs the default
# ./config.toml works.
ENV RUST_LOG=info
CMD ["/usr/local/bin/aoe2-tournament-bot"]
