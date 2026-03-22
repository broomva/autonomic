# Multi-stage build for autonomicd homeostasis controller
# Clones sibling dependencies (aiOS, lago, vigil) and builds the workspace

FROM rust:1-bookworm AS builder

WORKDIR /build

# Clone sibling dependencies (ADD forces cache bust on new commits)
ADD https://api.github.com/repos/broomva/lago/git/refs/heads/main /tmp/lago-ref
RUN git clone --depth 1 https://github.com/broomva/aiOS.git ../aiOS && \
    git clone --depth 1 https://github.com/broomva/lago.git ../lago && \
    git clone --depth 1 https://github.com/broomva/vigil.git ../vigil

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build release binary
RUN cargo build --release -p autonomicd

# Runtime stage
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash autonomic

COPY --from=builder /build/target/release/autonomicd /usr/local/bin/autonomicd

USER autonomic
WORKDIR /home/autonomic

ENV RUST_LOG=info
EXPOSE 3002

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD curl -f http://localhost:3002/health || exit 1

CMD ["autonomicd", "--bind", "0.0.0.0:3002"]
