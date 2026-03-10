################################################################################
# gvproxy builder
################################################################################

FROM golang:1.25 AS gvproxy-builder

WORKDIR /

RUN git clone --depth 1 --branch v0.7.4 https://github.com/containers/gvisor-tap-vsock.git
RUN cd gvisor-tap-vsock && CGO_ENABLED=0 GOARCH=amd64 GOOS=linux go build -ldflags '-extldflags "-static"' -o bin/gvproxy-linux-amd64 ./cmd/gvproxy

################################################################################
# control plane builder
################################################################################

FROM rust:1.88-slim AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y musl-tools \
    && rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml

RUN cargo build --release -p control-plane --target x86_64-unknown-linux-musl

COPY crates/control-plane/src crates/control-plane/src
RUN find crates/control-plane/src -name "*.rs" | xargs touch \
    && cargo build --release -p control-plane --target x86_64-unknown-linux-musl

################################################################################
# runtime
################################################################################

FROM alpine:3.20 AS runtime

RUN apk update && apk upgrade
RUN apk --no-cache add curl ca-certificates

COPY --from=gvproxy-builder /gvisor-tap-vsock/bin/gvproxy-linux-amd64 /app/gvproxy
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/control-plane /app/control-plane
RUN chmod +x /app/gvproxy /app/control-plane

EXPOSE 443
EXPOSE 9090

CMD ["/app/control-plane"]
