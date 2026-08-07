
################################################################################
# gvproxy builder
################################################################################

FROM golang:1.25 AS gvproxy-builder

WORKDIR /

# v0.8.7+ provides -ec2-metadata-access for IMDSv2 over TAP (see containers/gvisor-tap-vsock#512).
RUN git clone --depth 1 --branch v0.8.7 https://github.com/containers/gvisor-tap-vsock.git
RUN cd gvisor-tap-vsock && CGO_ENABLED=0 GOARCH=amd64 GOOS=linux go build -ldflags '-extldflags "-static"' -o bin/gvproxy-linux-amd64 ./cmd/gvproxy

################################################################################
# control-plane builder
################################################################################

FROM rust:1.95-slim AS control-plane-builder

WORKDIR /build

RUN apt-get update && apt-get install -y musl-tools \
    && rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/config/Cargo.toml crates/config/Cargo.toml
COPY crates/data-plane/Cargo.toml crates/data-plane/Cargo.toml
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml
COPY crates/telemetry/Cargo.toml crates/telemetry/Cargo.toml
COPY crates/verify/Cargo.toml crates/verify/Cargo.toml
COPY crates/sdk/Cargo.toml crates/sdk/Cargo.toml
COPY packages/node/Cargo.toml packages/node/Cargo.toml
COPY samples/hello/Cargo.toml samples/hello/Cargo.toml
COPY samples/wallet/Cargo.toml samples/wallet/Cargo.toml

# Stub sources so workspace members parse (control-plane depends on config/telemetry;
# other crates are workspace members only).
RUN mkdir -p crates/cli/src crates/control-plane/src crates/data-plane/src crates/data-plane/benches \
    crates/telemetry/src crates/verify/src crates/sdk/src \
    packages/node/src \
    samples/hello/src samples/wallet/src \
    && echo 'fn main() {}' > crates/cli/src/main.rs \
    && echo 'fn main() {}' > crates/data-plane/src/main.rs \
    && echo 'fn main() {}' > crates/control-plane/src/main.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/crypto.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/ingress.rs \
    && echo 'fn main() {}' > crates/data-plane/benches/tls_handshake.rs \
    && echo 'fn main() {}' > samples/hello/src/main.rs \
    && echo 'fn main() {}' > samples/wallet/src/main.rs \
    && touch crates/telemetry/src/lib.rs \
    && touch crates/verify/src/lib.rs \
    && touch crates/sdk/src/lib.rs \
    && touch packages/node/src/lib.rs

COPY crates/config/src crates/config/src
COPY crates/telemetry/src crates/telemetry/src
COPY crates/control-plane/src crates/control-plane/src

RUN cargo build --release -p control-plane --target x86_64-unknown-linux-musl

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

COPY --from=control-plane-builder /build/target/x86_64-unknown-linux-musl/release/control-plane /app/control-plane
COPY --from=gvproxy-builder /gvisor-tap-vsock/bin/gvproxy-linux-amd64 /app/gvproxy
RUN chmod +x /app/control-plane /app/gvproxy

ENV GVPROXY_BIN=/app/gvproxy

WORKDIR /app

EXPOSE 80 443

CMD ["/app/control-plane"]
