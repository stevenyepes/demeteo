# syntax=docker/dockerfile:1
# Build context is the repository root (compose sets `context: ../..`). Run
# from the root:
#   docker build -f deploy/hub/Dockerfile -t demeteo-hub .
#
# The COPYs below are an allow-list, not `COPY . .`: the context root holds
# `deploy/hub/.env` with the live database password and OpenBao token, and a
# whole-context copy bakes them into the builder layer and the build cache.
# Cargo resolves the whole workspace, so every member's manifest and targets
# must be present (`src-tauri/` included) even though only `demeteo-hub` is
# compiled. `Dockerfile.dockerignore` excludes `.env` files and `deploy/` as
# the second fence.
#
# The builder tag tracks `rust-toolchain.toml`; bump them together. The suffix
# pins the Debian release so the binary links against the glibc of the runtime
# stage below — change both or neither.
FROM rust:1.97.0-bookworm AS builder

# aws-lc-sys (rustls' crypto provider) compiles C and wants cmake.
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake pkg-config \
    && rm -rf /var/lib/apt/lists/*

# The root `.cargo/config.toml` points `target-dir` at `src-tauri/target`, a
# path that is excluded from this context and meaningless here. The env var
# outranks the config file, so the build lands in the cache mount below.
ENV CARGO_TARGET_DIR=/build/target

WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY .cargo ./.cargo
COPY crates ./crates
COPY src-tauri ./src-tauri

# A cache mount is not part of the layer, so the binary is copied out of it
# here; the runtime stage must copy from /usr/local/bin, never /build/target.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/build/target \
    cargo build --locked --release -p demeteo-hub \
    && cp /build/target/release/demeteo-hub /usr/local/bin/demeteo-hub

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --no-create-home --uid 10001 hub

COPY --from=builder /usr/local/bin/demeteo-hub /usr/local/bin/demeteo-hub

USER hub
ENTRYPOINT ["/usr/local/bin/demeteo-hub"]
