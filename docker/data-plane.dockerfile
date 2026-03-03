FROM rust:1.85-slim AS builder

WORKDIR /build

# Cache deps by copying manifests first
COPY Cargo.toml Cargo.toml
COPY crates/data-plane/Cargo.toml crates/data-plane/Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml

# TODO: Stub sources so cargo can resolve the workspace
RUN mkdir -p crates/data-plane/src crates/control-plane/src crates/shared/src \
    && echo 'fn main(){}' > crates/data-plane/src/main.rs \
    && echo 'fn main(){}' > crates/control-plane/src/main.rs \
    && echo '' > crates/shared/src/lib.rs

RUN cargo build --release -p data-plane

# Now copy real sources and rebuild only the changed crate
COPY crates/data-plane/src crates/data-plane/src
COPY crates/shared/src crates/shared/src
RUN touch crates/data-plane/src/main.rs && cargo build --release -p data-plane

# ── Runtime image ──────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

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
    # Allow binding to port 53 without root
    && setcap 'cap_net_bind_service=+ep' /app/data-plane

COPY docker/data-plane-entrypoint.sh /entrypoint.sh
RUN chmod +x /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
