# syntax=docker/dockerfile:1.7

################################################################################
# build base
################################################################################

FROM rust:1.95-slim AS build-base

WORKDIR /build

ENV CARGO_INCREMENTAL=0

RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools \
    make \
    perl \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl

################################################################################
# dependencies — compile against workspace stubs
################################################################################

FROM build-base AS dependencies

ARG FEATURES=""

COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/config/Cargo.toml         crates/config/Cargo.toml
COPY crates/data-plane/Cargo.toml     crates/data-plane/Cargo.toml
COPY crates/cli/Cargo.toml            crates/cli/Cargo.toml
COPY crates/telemetry/Cargo.toml      crates/telemetry/Cargo.toml
COPY crates/verify/Cargo.toml         crates/verify/Cargo.toml
COPY crates/sdk/Cargo.toml            crates/sdk/Cargo.toml
COPY packages/node/Cargo.toml         packages/node/Cargo.toml
COPY samples/hello/Cargo.toml         samples/hello/Cargo.toml
COPY samples/wallet/Cargo.toml        samples/wallet/Cargo.toml

# Stub workspace members so this stage compiles only external dependencies.
RUN mkdir -p crates/cli/src crates/control-plane/src crates/data-plane/src crates/config/src \
    crates/telemetry/src crates/verify/src crates/sdk/src \
    crates/data-plane/benches \
    packages/node/src \
    samples/hello/src samples/wallet/src \
    && echo 'fn main() {}' > crates/cli/src/main.rs \
    && echo 'fn main() {}' > crates/control-plane/src/main.rs \
    && echo 'fn main() {}' > crates/data-plane/src/main.rs \
    && touch crates/data-plane/src/lib.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/crypto.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/ingress.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/tls_handshake.rs \
    && echo 'fn main() {}' > samples/hello/src/main.rs \
    && echo 'fn main() {}' > samples/wallet/src/main.rs \
    && touch crates/config/src/lib.rs \
    && touch crates/telemetry/src/lib.rs \
    && touch crates/verify/src/lib.rs \
    && touch crates/sdk/src/lib.rs \
    && touch packages/node/src/lib.rs

# Keep compiled dependencies in this layer so both local and remote BuildKit
# caches retain them. Registry data uses cache mounts to avoid bloating the layer.
RUN --mount=type=cache,id=data-plane-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=data-plane-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    if [ -n "$FEATURES" ]; then \
        cargo build --locked --release -p data-plane --target x86_64-unknown-linux-musl --features "$FEATURES"; \
    else \
        cargo build --locked --release -p data-plane --target x86_64-unknown-linux-musl; \
    fi

################################################################################
# builder — compile real workspace sources using cached dependencies
################################################################################

FROM dependencies AS builder

ARG FEATURES=""

COPY crates/config/src      crates/config/src
COPY crates/telemetry/src   crates/telemetry/src
COPY crates/data-plane/src  crates/data-plane/src
COPY crates/data-plane/benches crates/data-plane/benches

RUN --mount=type=cache,id=data-plane-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=data-plane-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    cargo clean --release --target x86_64-unknown-linux-musl \
        -p config -p telemetry -p data-plane \
    && if [ -n "$FEATURES" ]; then \
        cargo build --locked --release -p data-plane --target x86_64-unknown-linux-musl --features "$FEATURES"; \
    else \
        cargo build --locked --release -p data-plane --target x86_64-unknown-linux-musl; \
    fi \
    && mkdir -p /out \
    && cp target/x86_64-unknown-linux-musl/release/data-plane /out/data-plane

################################################################################
# runtime
################################################################################

FROM alpine:3.20 AS runtime

ARG GIT_SHA=unknown
ARG VERSION=dev
LABEL org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.version="${VERSION}" \
      io.nitrum.git.sha="${GIT_SHA}"

# Nitro enclave kernels expose legacy netfilter tables reliably; Alpine's default
# `iptables` (nf_tables) often fails inside the EIF (see Evervault installer).
# The `iptables-legacy` package installs the binaries under /sbin; point the
# default `iptables`/`ip6tables` names at the legacy backend.
RUN apk add --no-cache iproute2 iptables iptables-legacy \
    && ln -sf /sbin/iptables-legacy /usr/sbin/iptables \
    && ln -sf /sbin/ip6tables-legacy /usr/sbin/ip6tables

COPY --from=builder /out/data-plane /app/data-plane
RUN chmod +x /app/data-plane

EXPOSE 443

CMD ["/app/data-plane"]