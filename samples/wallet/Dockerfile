# Build context must be the Nitrum repository root.
# `nitrum build` detects the workspace and sets this automatically for in-repo samples.
#
# The data-plane runtime image is Alpine/musl, so the app must be linked for
# `x86_64-unknown-linux-musl`.
ARG DATA_PLANE_IMAGE=ghcr.io/matzapata/nitrum/data-plane:latest
ARG RUST_IMAGE=rust:1.95-bookworm

FROM --platform=linux/amd64 ${RUST_IMAGE} AS builder

WORKDIR /src

RUN apt-get update && apt-get install -y --no-install-recommends musl-tools \
    && rm -rf /var/lib/apt/lists/* \
    && rustup target add x86_64-unknown-linux-musl

# Minimal workspace so `crates/sdk` workspace inheritance resolves without the full monorepo.
RUN printf '%s\n' \
  '[workspace]' \
  'members = ["crates/sdk", "samples/hello"]' \
  'resolver = "2"' \
  '' \
  '[workspace.package]' \
  'version = "0.1.0"' \
  'edition = "2024"' \
  'rust-version = "1.95"' \
  'license = "MIT"' \
  '' \
  '[workspace.lints.rust]' \
  'dead_code = "allow"' \
  '' \
  '[workspace.lints.clippy]' \
  'all = { level = "warn", priority = -1 }' \
  > Cargo.toml

COPY crates/sdk ./crates/sdk
COPY samples/wallet ./samples/wallet
RUN cargo build --release -p wallet-enclave --target x86_64-unknown-linux-musl

FROM --platform=linux/amd64 ${DATA_PLANE_IMAGE}
WORKDIR /app
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/wallet-enclave /app/wallet-enclave
COPY samples/wallet/nitrum.toml /app/nitrum.toml
CMD ["/app/data-plane", "--config", "/app/nitrum.toml"]
