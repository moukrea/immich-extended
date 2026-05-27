# syntax=docker/dockerfile:1.7

# Multi-stage build for the immich-extended server binary.
# Stage 1: cargo-chef base image used by planner and builder.
# Stage 2: planner extracts a dependency-only recipe.
# Stage 3: builder cooks the deps from the recipe, then compiles the binary.
# Stage 4: frontend bundles the SolidJS app via Vite (node:22-alpine).
# Stage 5: minimal Debian runtime image with the binary + migrations + dist.

FROM rust:1.91-slim AS chef
WORKDIR /app
RUN cargo install cargo-chef --locked --version 0.1.71

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
ENV SQLX_OFFLINE=true
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY migrations ./migrations
COPY .sqlx ./.sqlx
RUN cargo build --release -p immich-extended-server --bin immich-extended

FROM node:22-alpine AS frontend
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

FROM debian:trixie-slim AS runtime
ARG ONNXRUNTIME_VERSION=1.22.0
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget ffmpeg \
    && rm -rf /var/lib/apt/lists/* \
    && wget -qO /tmp/onnxruntime.tgz \
        "https://github.com/microsoft/onnxruntime/releases/download/v${ONNXRUNTIME_VERSION}/onnxruntime-linux-x64-${ONNXRUNTIME_VERSION}.tgz" \
    && tar -xzf /tmp/onnxruntime.tgz -C /tmp \
    && cp -P /tmp/onnxruntime-linux-x64-${ONNXRUNTIME_VERSION}/lib/libonnxruntime*.so* /usr/local/lib/ \
    && ldconfig \
    && rm -rf /tmp/onnxruntime*
WORKDIR /app
COPY --from=builder /app/target/release/immich-extended /usr/local/bin/immich-extended
COPY migrations/ /app/migrations/
COPY --from=frontend /web/dist /app/web/dist
ENV DATA_DIR=/data \
    HTTP_BIND=0.0.0.0:8080 \
    WEB_DIST_DIR=/app/web/dist \
    ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -qO- http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["/usr/local/bin/immich-extended"]
