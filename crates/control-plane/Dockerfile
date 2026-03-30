
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

FROM rust:1.92-slim AS control-plane-builder

WORKDIR /build

RUN apt-get update && apt-get install -y musl-tools \
    && rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml
COPY crates/data-plane/Cargo.toml crates/data-plane/Cargo.toml
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml

# Stub sources so workspace members parse (control-plane depends on shared; cli/data-plane are workspace members).
RUN mkdir -p crates/cli/src crates/control-plane/src crates/data-plane/src \
&& echo 'fn main() {}' > crates/cli/src/main.rs \
&& echo 'fn main() {}' > crates/data-plane/src/main.rs \
&& echo 'fn main() {}' > crates/control-plane/src/main.rs

COPY crates/shared/src crates/shared/src
COPY crates/control-plane/src crates/control-plane/src

RUN cargo build --release -p control-plane --target x86_64-unknown-linux-musl

################################################################################
# runtime 
################################################################################

FROM amazonlinux:2 AS runtime

RUN amazon-linux-extras install aws-nitro-enclaves-cli -y \
    && yum install -y curl ca-certificates \
    && yum clean all

COPY --from=control-plane-builder /build/target/x86_64-unknown-linux-musl/release/control-plane /app/control-plane
COPY --from=gvproxy-builder /gvisor-tap-vsock/bin/gvproxy-linux-amd64 /app/gvproxy
RUN chmod +x /app/control-plane /app/gvproxy

ENV GVPROXY_BIN=/app/gvproxy

EXPOSE 80 443

CMD ["/app/control-plane"]
