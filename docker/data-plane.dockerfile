FROM rust:1.88-slim AS builder

ARG FEATURES=""

WORKDIR /build

RUN apt-get update && apt-get install -y musl-tools \
    && rustup target add x86_64-unknown-linux-musl

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

RUN cargo build --release -p data-plane --target x86_64-unknown-linux-musl ${FEATURES:+--features $FEATURES}

# Copy real sources and touch every file cargo tracks to bust its mtime cache
COPY crates/data-plane/src crates/data-plane/src
COPY crates/shared/src crates/shared/src
RUN find crates/data-plane/src crates/shared/src -name "*.rs" | xargs touch \
    && cargo build --release -p data-plane --target x86_64-unknown-linux-musl ${FEATURES:+--features $FEATURES}

# ── Runtime image ──────────────────────────────────────────────────────────────
FROM --platform=linux/amd64 public.ecr.aws/amazonlinux/amazonlinux:2

RUN yum install -y iproute && yum clean all

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/data-plane /app/data-plane

ENTRYPOINT ["/app/data-plane"]
