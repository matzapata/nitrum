# Releases and versioning

## Git tags and GitHub Releases

- Tags use [Semantic Versioning](https://semver.org/): `vMAJOR.MINOR.PATCH` (for example `v0.2.1`).
- Pushing a `v*` tag runs the [Release workflow](../.github/workflows/release.yml), which:
  1. Runs the full [CI workflow](../.github/workflows/ci.yml) (Rust, multi-platform `nitrum-node` prebuilds, `cargo-deny`, CHANGELOG check).
  2. Pushes Docker images to GHCR (`control-plane`, `data-plane`, `data-plane:*-local`, `nitro-cli`).
  3. Publishes `nitrum-node` (and per-platform optional packages) to npm from the CI prebuilds.
  4. Builds and attaches multi-platform CLI binaries to the GitHub Release.

Before tagging, add a section to [CHANGELOG.md](../CHANGELOG.md) for the version (see `## [0.1.0]` format). CI verifies this for tag builds. Set the npm package version via the git tag (`v1.2.3` → `nitrum-node@1.2.3`).

### Secrets

| Secret | Used by |
|--------|---------|
| `NPM_TOKEN` | Automation token (or granular token) that can publish `nitrum-node` and `nitrum-node-*` platform packages |

## Rust workspace

- Crate versions are bumped together with the release tag.
- **MSRV** is `rust-version` in the root [Cargo.toml](../Cargo.toml) (currently **1.95**). CI and [rust-toolchain.toml](../rust-toolchain.toml) use the same version.
- MSRV may increase in a **minor** release with notice in the CHANGELOG.

## `nitrum.toml` compatibility

| Change type | SemVer bump | Examples |
|-------------|-------------|----------|
| Documentation-only or looser validation | Patch | Clarify error messages, accept additional optional formats |
| New optional keys with serde defaults | Minor | New `[cloud]` / `[local]` keys with defaults (`template`, `instance_managed_policy_arns`) |
| Rename/remove keys, new required fields, stricter validation | Major | Rename `runtime.data_plane`, require a new mandatory section |

While the project is **0.x**, breaking `nitrum.toml` changes may still appear in **minor** releases until **1.0**. After 1.0, follow the table strictly.

Config schema and validation live in [`crates/config`](../crates/config/).

## CLI compatibility

- Command names, flags, and default behavior follow SemVer.
- Breaking changes to commands or output require a **major** version bump.
- Pre-1.0 releases may introduce breaking CLI changes in minors; check the CHANGELOG.

## Supply chain

- **cargo-deny** runs on every CI build ([`deny.toml`](../deny.toml)).
- **Dependabot** opens weekly update PRs for Rust (`Cargo.lock`), npm, and GitHub Actions.

## Docker image provenance

Release images include OCI labels:

- `org.opencontainers.image.revision` — full Git commit SHA
- `org.opencontainers.image.version` — release tag (for example `v0.1.0`)
- `io.nitrum.git.sha` — same as revision (Nitrum-specific)

Pin images by digest in `nitrum.toml` when possible. See [usage.md — Runtime image provenance](usage.md#runtime-image-provenance).

## Optional: required checks on `master`

In repository settings, you can require these CI jobs before merge:

- Rust Format, Rust Lint, Rust Check, Rust Test
- nitrum-node (per-platform matrix)
- Supply chain (cargo-deny)
- CHANGELOG (on tag pushes only)

Tag releases are additionally gated inside the Release workflow via `needs: ci`.
