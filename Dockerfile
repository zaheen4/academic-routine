# Multi-stage Docker build for ultra-small container deployment
FROM rust:1-slim AS builder

WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests

# Build optimized production binary
RUN cargo build --release

# Minimal runtime image
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /usr/src/app/target/release/academic-routine /usr/local/bin/academic-routine

# Default command launches Telegram bot daemon
CMD ["academic-routine", "bot"]
