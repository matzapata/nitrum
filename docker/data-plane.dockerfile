FROM rust:1.88-slim AS builder

ARG FEATURES=""

WORKDIR /build

# Cache deps by copying manifests first
COPY Cargo.toml Cargo.toml
COPY crates/data-plane/Cargo.toml crates/data-plane/Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml

# Stub sources so cargo can resolve the workspace and pre-fetch dependencies
RUN mkdir -p crates/data-plane/src crates/control-plane/src crates/shared/src \
    && echo 'fn main(){}' > crates/data-plane/src/main.rs \
    && echo 'fn main(){}' > crates/control-plane/src/main.rs \
    && echo '' > crates/shared/src/lib.rs

RUN cargo build --release -p data-plane ${FEATURES:+--features $FEATURES}

# Copy real sources and touch every file cargo tracks to bust its mtime cache
COPY crates/data-plane/src crates/data-plane/src
COPY crates/shared/src crates/shared/src
RUN find crates/data-plane/src crates/shared/src -name "*.rs" | xargs touch \
    && cargo build --release -p data-plane ${FEATURES:+--features $FEATURES}

# ── Runtime image ──────────────────────────────────────────────────────────────
FROM --platform=linux/amd64 debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    iptables \
    curl \
    ca-certificates \
    libcap2-bin \
    && rm -rf /var/lib/apt/lists/*

# Create a non-root user for the data-plane proxy so iptables can exempt its traffic
RUN useradd -u 1500 -M -s /bin/sh dataplane

COPY --from=builder /build/target/release/data-plane /app/data-plane
RUN chmod +x /app/data-plane \
    # Allow binding to privileged ports (e.g. DNS on 53) without full root.
    && setcap 'cap_net_bind_service=+ep' /app/data-plane

ENTRYPOINT ["/app/data-plane"]
