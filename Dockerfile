# multi-stage Dockerfile for MyNewsLens single-binary Rust application
#
# Notes:
# - Builds the Rust binary in a Rust official image (builder stage).
# - Produces a minimal runtime image based on Debian slim with a non-root user.
# - Use Docker Buildx for multi-arch builds (ARMv7/RPI / amd64) when building for Raspberry Pi.
#
# Usage examples:
#  - Build locally:    docker build -t mynewslens:latest .
#  - Run (with config & data): docker run -v $(pwd)/config.toml:/config/config.toml -v $(pwd)/data:/data -p 8000:8000 mynewslens:latest
#  - Build multi-arch (example): docker buildx build --platform linux/arm/v7,linux/amd64 -t mynewslens:latest --push .

# --------------------
# Builder stage
# --------------------
ARG RUST_IMAGE=rust:1.72-slim
FROM ${RUST_IMAGE} as builder

# Allow overriding target (useful for cross compile with buildx)
ARG TARGET
ENV CARGO_HOME=/usr/local/cargo
ENV RUSTFLAGS="-C target-cpu=native"

# Install build dependencies (kept reasonably small)
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    ca-certificates \
    curl \
    libssl-dev \
    git \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /work

# Copy manifests first to leverage Docker layer cache
COPY Cargo.toml Cargo.lock ./
# If there are workspace members, copy their manifests too (optional)
# COPY common/Cargo.toml common/Cargo.toml
# NOTE: the below `cargo fetch` improves build caching for dependencies
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/root/.cargo \
    cargo fetch --locked

# Copy the full source
COPY . .

# Build release (for specific target if provided)
# If TARGET is set (via --build-arg TARGET=...), cargo will build for that target.
RUN if [ -z "$TARGET" ]; then \
      cargo build --release --bin mynewslens; \
    else \
      rustup target add ${TARGET} || true; \
      cargo build --release --target ${TARGET} --bin mynewslens; \
    fi

# --------------------
# Runtime stage
# --------------------
# Use a small but compatible base image. If you need even smaller, consider distroless.
FROM debian:bookworm-slim AS runtime

# Create a non-root user for security
ARG APP_USER=mynews
ARG APP_GROUP=mynews
RUN groupadd -r ${APP_GROUP} && useradd -r -g ${APP_GROUP} -d /nonexistent -s /usr/sbin/nologin ${APP_USER}

# Install certificates and optionally curl for healthchecks
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
  && rm -rf /var/lib/apt/lists/*

# Create directories for config and data (to be mounted)
RUN mkdir -p /config /data /usr/local/bin /var/log/mynewslens \
  && chown -R ${APP_USER}:${APP_GROUP} /data /config /var/log/mynewslens

# Copy binary from builder
ARG TARGET
# If TARGET was used in build, binary will be under target/<TARGET>/release/mynewslens
# Otherwise under target/release/mynewslens
COPY --from=builder /work/target/release/mynewslens /usr/local/bin/mynewslens
# Fallback: try copy target/<target>/release if the previous path wasn't used during build
COPY --from=builder /work/target/*/release/mynewslens /usr/local/bin/mynewslens || true

RUN chown ${APP_USER}:${APP_GROUP} /usr/local/bin/mynewslens && chmod 755 /usr/local/bin/mynewslens

USER ${APP_USER}
WORKDIR /home/${APP_USER}

# Expose default HTTP port (configurable)
EXPOSE 8000

# Default entrypoint & args:
# - By default the binary will look for /config/config.toml
# - The binary supports flags: --config, --no-worker, --worker-only, --log-level
ENTRYPOINT ["/usr/local/bin/mynewslens"]
CMD ["--config", "/config/config.toml"]

# Healthcheck is intentionally left out (compose file may define healthcheck).
# If desired, add:
# HEALTHCHECK --interval=60s --timeout=10s --start-period=20s CMD curl -f http://localhost:8000/health || exit 1
