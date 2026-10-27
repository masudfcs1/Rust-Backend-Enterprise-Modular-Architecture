# syntax=docker/dockerfile:1
FROM rust:1.99.1-bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --locked --release && \
    cp target/release/enterprise-rust-backend /usr/local/bin/enterprise-rust-backend

FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system app && useradd --system --gid app --home-dir /app app
WORKDIR /app
COPY --from=builder /usr/local/bin/enterprise-rust-backend /usr/local/bin/enterprise-rust-backend
USER app
ENV HTTP_HOST=0.0.0.0 HTTP_PORT=3000
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=3s --start-period=10s --retries=3 \
    CMD curl --fail --silent http://127.0.0.1:3000/health/ready || exit 1
ENTRYPOINT ["enterprise-rust-backend"]
CMD ["serve"]
