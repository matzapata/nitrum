# syntax=docker/dockerfile:1.7

################################################################################
# gvproxy builder
################################################################################

FROM golang:1.25 AS gvproxy-builder

WORKDIR /

# v0.8.7+ provides -ec2-metadata-access for IMDSv2 over TAP (see containers/gvisor-tap-vsock#512).
RUN git clone --depth 1 --branch v0.8.7 https://github.com/containers/gvisor-tap-vsock.git
RUN cd gvisor-tap-vsock && CGO_ENABLED=0 GOARCH=amd64 GOOS=linux go build -ldflags '-extldflags "-static"' -o bin/gvproxy-linux-amd64 ./cmd/gvproxy

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
COPY examples/hello/Cargo.toml         examples/hello/Cargo.toml
COPY examples/wallet/Cargo.toml        examples/wallet/Cargo.toml

# Stub workspace members so this stage compiles only external dependencies.
# control-plane depends on config/telemetry; other crates are workspace members only.
RUN mkdir -p crates/cli/src crates/control-plane/src crates/data-plane/src crates/config/src \
    crates/telemetry/src crates/verify/src crates/sdk/src \
    crates/data-plane/benches \
    packages/node/src \
    examples/hello/src examples/wallet/src \
    && echo 'fn main() {}' > crates/cli/src/main.rs \
    && echo 'fn main() {}' > crates/control-plane/src/main.rs \
    && echo 'fn main() {}' > crates/data-plane/src/main.rs \
    && touch crates/data-plane/src/lib.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/crypto.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/ingress.rs \
    && echo 'fn main() {}' > examples/hello/src/main.rs \
    && echo 'fn main() {}' > examples/wallet/src/main.rs \
    && touch crates/config/src/lib.rs \
    && touch crates/telemetry/src/lib.rs \
    && touch crates/verify/src/lib.rs \
    && touch crates/sdk/src/lib.rs \
    && touch packages/node/src/lib.rs

# Keep compiled dependencies in this layer so both local and remote BuildKit
# caches retain them. Registry data uses cache mounts to avoid bloating the layer.
RUN --mount=type=cache,id=control-plane-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=control-plane-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    cargo build --locked --release -p control-plane --target x86_64-unknown-linux-musl

################################################################################
# builder — compile real workspace sources using cached dependencies
################################################################################

FROM dependencies AS builder

COPY crates/config/src        crates/config/src
COPY crates/telemetry/src     crates/telemetry/src
COPY crates/control-plane/src  crates/control-plane/src

RUN --mount=type=cache,id=control-plane-cargo-registry,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,id=control-plane-cargo-git,target=/usr/local/cargo/git,sharing=locked \
    cargo clean --release --target x86_64-unknown-linux-musl \
        -p config -p telemetry -p control-plane \
    && cargo build --locked --release -p control-plane --target x86_64-unknown-linux-musl \
    && mkdir -p /out \
    && cp target/x86_64-unknown-linux-musl/release/control-plane /out/control-plane

################################################################################
# runtime
################################################################################

FROM amazonlinux:2 AS runtime

ARG GIT_SHA=unknown
ARG VERSION=dev
LABEL org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.version="${VERSION}" \
      io.nitrum.git.sha="${GIT_SHA}"

RUN amazon-linux-extras install aws-nitro-enclaves-cli -y \
    && yum install -y curl ca-certificates \
    && yum clean all

COPY --from=builder /out/control-plane /app/control-plane
COPY --from=gvproxy-builder /gvisor-tap-vsock/bin/gvproxy-linux-amd64 /app/gvproxy
RUN chmod +x /app/control-plane /app/gvproxy

ENV GVPROXY_BIN=/app/gvproxy

WORKDIR /app

EXPOSE 80 443

CMD ["/app/control-plane"]
