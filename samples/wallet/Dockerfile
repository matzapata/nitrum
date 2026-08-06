# Build context must be the Nitrum repository root.
# `nitrum build` detects the workspace and sets this automatically for in-repo samples.
ARG DATA_PLANE_IMAGE=ghcr.io/matzapata/nitrum/data-plane:latest-dev
ARG RUST_IMAGE=rust:1.95-bookworm

FROM --platform=linux/amd64 ${RUST_IMAGE} AS builder
WORKDIR /src
# Minimal workspace so `crates/sdk` workspace inheritance resolves without the full monorepo.
RUN printf '%s\n' \
  '[workspace]' \
  'members = ["crates/sdk", "samples/wallet"]' \
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
RUN cargo build --release -p wallet-enclave

FROM --platform=linux/amd64 ${DATA_PLANE_IMAGE}
WORKDIR /app
COPY --from=builder /src/target/release/wallet-enclave /app/wallet-enclave
COPY samples/wallet/nitrum.toml /app/nitrum.toml
CMD ["/app/data-plane", "--config", "/app/nitrum.toml"]
