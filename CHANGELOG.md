# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- CI: `cargo test --all-features`, `packages/nitrum-node` lint/test, `cargo-deny`, Dependabot.
- Release gates: tag releases require green CI before GHCR push and GitHub Release assets.
- OCI image labels: `org.opencontainers.image.revision`, `io.nitrum.git.sha`, and version metadata on runtime images.
- MSRV pinned to Rust 1.95 in the workspace `Cargo.toml`.

## [0.1.0] - 2026-05-25

### Added

- Initial public beta: Nitrum CLI, control-plane, data-plane, `nitrum.toml` config, local and cloud deploy flows.
- `packages/nitrum-node` for attestation verification in Node.js.
- Sample projects under `samples/hello` and `samples/wallet`.
