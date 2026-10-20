# Multi-stage Docker build for standalone container deployment
FROM rust:1-slim AS builder

WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests

# Build optimized production binary
RUN cargo build --release

# Production runtime image with headless browser & virtual display capabilities
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    chromium \
    xvfb \
    python3 \
    python3-pip \
    python3-venv \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Set up isolated Python virtual environment for scraper dependencies
COPY scraper /app/scraper
RUN python3 -m venv /app/.venv \
    && /app/.venv/bin/pip install --no-cache-dir -r /app/scraper/requirements.txt

COPY --from=builder /usr/src/app/target/release/academic-routine /usr/local/bin/academic-routine

# Default command launches Telegram bot daemon
CMD ["academic-routine", "bot"]
