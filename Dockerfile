# syntax=docker/dockerfile:1.7

# Multi-stage build for the immich-extended server binary.
# Stage 1: cargo-chef base image used by planner and builder.
# Stage 2: planner extracts a dependency-only recipe.
# Stage 3: builder cooks the deps from the recipe, then compiles the binary.
# Stage 4: minimal Debian runtime image with the binary + migrations.

FROM rust:1.91-slim AS chef
WORKDIR /app
RUN cargo install cargo-chef --locked --version 0.1.71

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
RUN cargo build --release -p immich-extended-server --bin immich-extended

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /app/target/release/immich-extended /usr/local/bin/immich-extended
COPY migrations/ /app/migrations/
ENV DATA_DIR=/data \
    HTTP_BIND=0.0.0.0:8080
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["/usr/local/bin/immich-extended"]
