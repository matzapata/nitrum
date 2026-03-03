FROM rust:1.85-slim AS builder

WORKDIR /build

# Cache deps by copying manifests first
COPY Cargo.toml Cargo.toml
COPY crates/data-plane/Cargo.toml crates/data-plane/Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml

# Stub sources so cargo can resolve the workspace
RUN mkdir -p crates/data-plane/src crates/control-plane/src crates/shared/src \
    && echo 'fn main(){}' > crates/data-plane/src/main.rs \
    && echo 'fn main(){}' > crates/control-plane/src/main.rs \
    && echo '' > crates/shared/src/lib.rs

RUN cargo build --release -p control-plane

# Now copy real sources and rebuild only the changed crate
COPY crates/control-plane/src crates/control-plane/src
COPY crates/shared/src crates/shared/src
RUN touch crates/control-plane/src/main.rs && cargo build --release -p control-plane

# ── Runtime image ──────────────────────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/control-plane /app/control-plane
RUN chmod +x /app/control-plane

EXPOSE 8181
EXPOSE 3031

CMD ["/app/control-plane"]
