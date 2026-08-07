ARG DATA_PLANE_IMAGE=ghcr.io/matzapata/nitrum/data-plane:latest
ARG RUST_IMAGE=rust:1.95-bookworm

################################################################################
# Builder — compile the Rust app using the data-plane runtime image
################################################################################

FROM --platform=linux/amd64 ${RUST_IMAGE} AS builder

WORKDIR /build

RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl

COPY Cargo.toml ./
COPY src ./src

################################################################################
# Runtime 
################################################################################

RUN cargo build --release --target x86_64-unknown-linux-musl

FROM --platform=linux/amd64 ${DATA_PLANE_IMAGE}

WORKDIR /app
COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/hello /app/hello
COPY nitrum.toml /app/nitrum.toml

CMD ["/app/data-plane", "--config", "/app/nitrum.toml"]
