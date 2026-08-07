# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `[cloud]` in `nitrum.toml` for CloudFormation knobs: `xray_tracing`, `log_retention_days`, `sns_alarm_topic_arn`, `safe_rolling`, `kms_administrator_role_arn` (ignored by `nitrum local`).
- `[scaling].instance_type` with a Nitro Enclave–capable EC2 allowlist and enclave CPU/RAM fit checks (default `m6i.xlarge`).
- Safe ASG rolling updates (`MinInstancesInService` / `PauseTime`), optional X-Ray export, configurable log retention, SNS alarms, HTTPS NLB health checks to `/.well-known/enclave/status`, and DynamoDB PITR + deletion protection when `--retain`.
- Deploy preflight warning when the caller identity does not match `cloud.kms_administrator_role_arn`.
- `crates/verify` — pure Rust AWS Nitro attestation verification (COSE, chain, PCR/nonce/age, TLS leaf hash bind).
- `crates/sdk` (`nitrum-sdk`) — in-enclave HTTP client for the data-plane crypto API (`encrypt` / `decrypt` / `random` / `kv` / `attestation`).
- `packages/node` (npm `nitrum-node`) — thin napi-rs bindings over `verify` (document verify + TLS leaf hash bind).
- Rust rewrites of `examples/hello` and `examples/wallet` using `nitrum-sdk`; `nitrum init` scaffolds the Rust hello example.
- CI: `cargo test --all-features`, multi-platform `nitrum-node` prebuilds (macOS/Linux/Windows), `cargo-deny`, Dependabot.
- Release gates: tag releases require green CI before GHCR push, `nitrum-node` npm publish (with platform optional packages), and GitHub Release CLI assets.
- OCI image labels: `org.opencontainers.image.revision`, `io.nitrum.git.sha`, and version metadata on runtime images.
- MSRV pinned to Rust 1.95 in the workspace `Cargo.toml`.

### Changed

- Default `[scaling].max_replicas` is `2` so safe rolling has headroom; `nitrum cloud deploy` validates `safe_rolling` against replica bounds.
- CloudFormation deploy always re-passes stack parameters (including KMS admin) so omitted values cannot silently reset to template defaults.
- Replaced TypeScript-only `packages/nitrum-node` with Rust-backed `packages/node` (standard napi-rs generated `index.js` / `index.d.ts`).
- Example enclave apps no longer require Node.js inside the EIF image.
- Tag releases publish `nitrum-node` to npm with per-platform optionalDependencies (no local Rust toolchain required for consumers).

### Removed

- `nitrum cloud deploy --kms-administrator-role-arn` — use `[cloud].kms_administrator_role_arn` in `nitrum.toml` instead.
- `[well_known]` (`enclave_status` / `enclave_attestation`) — `/.well-known/enclave/status` and `/.well-known/enclave/attestation` are always enabled (required for NLB health checks and remote attestation). Stale `[well_known]` keys in existing `nitrum.toml` files are ignored.

## [0.1.0] - 2026-05-25

### Added

- Initial public beta: Nitrum CLI, control-plane, data-plane, `nitrum.toml` config, local and cloud deploy flows.
- `packages/nitrum-node` for attestation verification in Node.js.
- Example projects under `examples/hello` and `examples/wallet`.
