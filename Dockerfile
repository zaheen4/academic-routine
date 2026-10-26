# Multi-stage Docker build for standalone container deployment
FROM rust:1.98-slim-bookworm AS builder

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
ENV PORT=8000

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    chromium \
    xvfb \
    xauth \
    python3 \
    && ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone \
    && rm -rf /var/lib/apt/lists/*

# Copy uv binary from official pinned image for fast, deterministic dependency management
COPY --from=ghcr.io/astral-sh/uv:0.13.5 /uv /bin/uv

# Create non-root application user
RUN useradd -m -u 1000 app && mkdir -p /app/output && chown -R app:app /app

WORKDIR /app

# Install scraper dependencies using uv
COPY pyproject.toml uv.lock ./
COPY scraper /app/scraper
RUN uv sync --frozen --no-dev && chown -R app:app /app

COPY --from=builder /usr/src/app/target/release/academic-routine /usr/local/bin/academic-routine

USER app

EXPOSE 8000

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
    CMD python3 -c "import urllib.request, os; urllib.request.urlopen('http://127.0.0.1:' + os.environ.get('PORT', '8000') + '/health')" || exit 1

# Default command launches Telegram bot daemon
CMD ["academic-routine", "bot"]
