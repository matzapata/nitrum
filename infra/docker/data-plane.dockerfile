################################################################################
# chef — installs cargo-chef once, reused by both planner and builder
################################################################################

FROM rust:1.92-slim AS chef

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools \
    make \
    perl \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl \
    && cargo install cargo-chef --locked

################################################################################
# planner — computes the recipe (dependency fingerprint)
################################################################################

FROM chef AS planner

COPY Cargo.toml Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/shared/Cargo.toml         crates/shared/Cargo.toml
COPY crates/data-plane/Cargo.toml     crates/data-plane/Cargo.toml
COPY crates/cli/Cargo.toml            crates/cli/Cargo.toml

# Stub all workspace members so `cargo chef prepare` can resolve the full graph
RUN mkdir -p crates/cli/src crates/control-plane/src crates/data-plane/src crates/shared/src \
    && echo 'fn main() {}' > crates/cli/src/main.rs \
    && echo 'fn main() {}' > crates/control-plane/src/main.rs \
    && echo 'fn main() {}' > crates/data-plane/src/main.rs \
    && touch crates/shared/src/lib.rs

RUN cargo chef prepare --recipe-path recipe.json

################################################################################
# builder — cooks deps (cached), then compiles real sources
################################################################################

FROM chef AS builder

ARG FEATURES=""

COPY --from=planner /build/recipe.json recipe.json

# This layer is cached as long as recipe.json (i.e. Cargo.toml/lock) is unchanged
RUN if [ -n "$FEATURES" ]; then \
        cargo chef cook --release -p data-plane --target x86_64-unknown-linux-musl --features "$FEATURES" --recipe-path recipe.json; \
    else \
        cargo chef cook --release -p data-plane --target x86_64-unknown-linux-musl --recipe-path recipe.json; \
    fi

COPY Cargo.toml Cargo.toml
COPY crates/shared/src      crates/shared/src
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml
COPY crates/data-plane/src  crates/data-plane/src
COPY crates/data-plane/Cargo.toml crates/data-plane/Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/cli/Cargo.toml  crates/cli/Cargo.toml

RUN if [ -n "$FEATURES" ]; then \
        cargo build --release -p data-plane --target x86_64-unknown-linux-musl --features "$FEATURES"; \
    else \
        cargo build --release -p data-plane --target x86_64-unknown-linux-musl; \
    fi

################################################################################
# runtime
################################################################################

FROM alpine:3.20 AS runtime

RUN apk update && apk upgrade && apk --no-cache add iproute2

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/data-plane /app/data-plane
RUN chmod +x /app/data-plane

EXPOSE 443
EXPOSE 9090

CMD ["/app/data-plane"]