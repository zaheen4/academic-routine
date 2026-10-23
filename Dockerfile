# Multi-stage Docker build for standalone container deployment
FROM rust:1-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src/app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY tests ./tests

# Build optimized production binary
RUN cargo build --release

# Production runtime image with headless browser & virtual display capabilities
FROM debian:bookworm-slim

ENV TZ=Asia/Dhaka
ENV ROUTINE_TIMEZONE_OFFSET=6

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    chromium \
    xvfb \
    xauth \
    python3 \
    && ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy uv binary from official image for fast, deterministic dependency management
COPY --from=ghcr.io/astral-sh/uv:latest /uv /bin/uv

# Install scraper dependencies using uv
COPY pyproject.toml uv.lock ./
COPY scraper /app/scraper
RUN uv sync --frozen --no-dev

COPY --from=builder /usr/src/app/target/release/academic-routine /usr/local/bin/academic-routine

# Default HTTP health check port for cloud platforms (Render)
ENV PORT=8000
EXPOSE 8000

# Default command launches Telegram bot daemon
CMD ["academic-routine", "bot"]
