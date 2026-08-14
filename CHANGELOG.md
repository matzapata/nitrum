# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `nitrum cloud eject` writes the bundled CloudFormation template to `infra/cloud-stack.yml` (optional `--output`; `--force` to overwrite).
- `[cloud].template` — project-relative CloudFormation YAML; deploy uses the bundled template when omitted. Deploy fails if `infra/cloud-stack.yml` exists without this key.
- `[cloud].instance_managed_policy_arns` — extra IAM managed policy ARNs on the EC2 instance role (in addition to SSM core).
- CloudFormation templates larger than 51,200 bytes are uploaded to the EIF bucket and deployed via `TemplateURL` (bucket policy allows CloudFormation `GetObject` on `cloudformation/*`, scoped with `aws:SourceAccount` and `aws:SourceArn`).
- `# nitrum-template-version:` marker and a skew warning when an ejected template does not match the CLI bundle.

## [0.2.0] - 2026-08-12

### Added

- Data-plane probes `[health_check]` and gates `GET /.well-known/enclave/status` on app readiness (`200` / `503`) so NLB stops sending traffic to unhealthy instances.
- `[cloud]` in `nitrum.toml` for CloudFormation knobs: `xray_tracing`, `log_retention_days`, `sns_alarm_topic_arn`, `safe_rolling`, `kms_administrator_role_arn` (ignored by `nitrum local`).
- `[scaling].instance_type` with a Nitro Enclave–capable EC2 allowlist and enclave CPU/RAM fit checks (default `m6i.xlarge`).
- Safe ASG rolling updates (`MinInstancesInService` / `PauseTime`), optional X-Ray export, configurable log retention, SNS alarms, HTTPS NLB health checks to `/.well-known/enclave/status`, and DynamoDB PITR + deletion protection when `--retain`.
- Deploy preflight warning when the caller identity does not match `cloud.kms_administrator_role_arn`.
- `[egress]` in-enclave DNS/TCP allowlist (iptables + DNS proxy); documented limits for UDP/IPv6 and best-effort filter tables on Nitro kernels.
- OpenTelemetry export from control-plane and data-plane via host ADOT collector (CloudWatch metrics/logs, optional X-Ray); injects `OTEL_*` into the user process when OTLP is enabled.
- `crates/verify` — pure Rust AWS Nitro attestation verification (COSE, chain, PCR/nonce/age, TLS leaf hash bind).
- `crates/sdk` (`nitrum-sdk`) — in-enclave HTTP client for the data-plane crypto API (`encrypt` / `decrypt` / `random` / `attestation`).
- `packages/node` (npm `nitrum-node`) — thin napi-rs bindings over `verify` (document verify + TLS leaf hash bind).
- Rust rewrites of `examples/hello` and `examples/wallet` using `nitrum-sdk`; `nitrum init` scaffolds the Rust hello example.
- Criterion micro-benches for data-plane crypto and ingress; k6 macro load harness for deployed Nitro enclaves (`tests/perf`).
- Manual AWS cloud e2e workflow; local Compose e2e GitHub Actions workflow (`e2e-local.yml`) with ACME/pebble gating and cleanup-on-exit script improvements.
- CI: `cargo test --all-features`, multi-platform `nitrum-node` prebuilds (macOS/Linux/Windows), `cargo-deny`, Dependabot.
- Release gates: tag releases require green CI before GHCR push, `nitrum-node` npm publish (with platform optional packages), and GitHub Release CLI assets.
- OCI image labels: `org.opencontainers.image.revision`, `io.nitrum.git.sha`, and version metadata on runtime images.
- MSRV pinned to Rust 1.95 in the workspace `Cargo.toml`.
- Networking documentation (`docs/networking.md`) and architecture diagram generation.

### Changed

- Default `[scaling].max_replicas` is `2` so safe rolling has headroom; `nitrum cloud deploy` validates `safe_rolling` against replica bounds.
- CloudFormation deploy always re-passes stack parameters (including KMS admin) so omitted values cannot silently reset to template defaults.
- Replaced TypeScript-only `packages/nitrum-node` with Rust-backed `packages/node` (standard napi-rs generated `index.js` / `index.d.ts`).
- Example enclave apps no longer require Node.js inside the EIF image.
- Tag releases publish `nitrum-node` to npm with per-platform optionalDependencies (no local Rust toolchain required for consumers).
- Data-plane crypto/storage refactored behind traits (`Dek`, storage backends) with in-memory test doubles and expanded unit coverage.
- Ingress proxy and related data-plane hot paths optimized (HTTP client reuse, buffering, vsock/networking polish).

### Removed

- `nitrum cloud deploy --kms-administrator-role-arn` — use `[cloud].kms_administrator_role_arn` in `nitrum.toml` instead.
- `[well_known]` (`enclave_status` / `enclave_attestation`) — `/.well-known/enclave/status` and `/.well-known/enclave/attestation` are always enabled (required for NLB health checks and remote attestation). Stale `[well_known]` keys in existing `nitrum.toml` files are ignored.

## [0.1.0] - 2026-05-25

### Added

- Initial public beta: Nitrum CLI, control-plane, data-plane, `nitrum.toml` config, local and cloud deploy flows.
- `packages/nitrum-node` for attestation verification in Node.js.
- Example projects under `examples/hello` and `examples/wallet`.
