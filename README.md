## Nitrum

> **⚠️ WARNING:** Nitrum is **work in progress** and **not ready for production use**. APIs, features, and security properties may change at any time.  
Please use for development, testing, and feedback only!

Nitrum is a **Rust** toolkit for running applications inside **AWS Nitro Enclaves**. It combines an in-enclave **data-plane**, a host **control-plane**, and a **`nitrum` CLI** that helps you scaffold projects, build EIF images, run a local Docker stack for testing, and deploy to AWS.

If you are familiar with platforms like [Evervault Enclaves](https://docs.evervault.com/enclaves) or [Nitriding daemon](https://github.com/brave/nitriding-daemon), Nitrum plays a similar role: it focuses on **TLS termination, attestation, and networking inside the enclave**, so your application code can stay as close as possible to “regular” HTTP services. On top of that, Nitrum adds utilities for attestation, encryption with key synchronization, and a CLI to manage the entire development workflow—from local testing to production deployment.

### Goals

- **Make Nitro Enclaves approachable**: sensible defaults, clear CLI workflows, and sample projects.
- **Keep sensitive work in-enclave**: TLS termination, attestation, and AWS KMS patterns are handled by the data-plane, not your app.
- **Integrate with AWS tooling**: Nitro CLI, IMDS, KMS, S3, CloudFormation, and IAM are first‑class concerns.

### Installation (prebuilt binary)

Tagged releases publish CLI binaries for **Linux x86_64**, **macOS Apple Silicon (aarch64)**, and **Windows x86_64** (same targets as [.github/workflows/release.yml](.github/workflows/release.yml)).

From a checkout:

```bash
./scripts/install-nitrum.sh
```

Or fetch and run the script from GitHub (pick a branch or tag you trust, for example `develop` or `v0.1.0`):

```bash
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum/develop/scripts/install-nitrum.sh | bash
```

Optional environment variables:

- **`NITRUM_VERSION`** — `latest` (default) or a tag such as `v0.1.0-beta.1`
- **`NITRUM_INSTALL_DIR`** — install directory (default: `~/.local/bin`)
- **`NITRUM_REPO`** — `owner/name` if you use a fork (default: `matzapata/nitrum`)

Example: install a specific release into `/usr/local/bin` (may require write permission):

```bash
export NITRUM_VERSION=v0.1.0-beta.1
export NITRUM_INSTALL_DIR=/usr/local/bin
curl -fsSL https://raw.githubusercontent.com/matzapata/nitrum/develop/scripts/install-nitrum.sh | sudo -E bash
```

Ensure the install directory is on your `PATH`.

### Installation (from source)

You can build and install the CLI directly from this repository:

```bash
cargo install --path crates/cli
nitrum --help
```

For a full list of commands, required AWS permissions, and `nitrum.toml` options, see the [usage documentation](docs/usage.md). 

### Documentation

- **Architecture** — how the control-plane, data-plane, TLS, attestation, and AWS integrations fit together:  
  [docs/architecture.md](docs/architecture.md)
- **Networking** — how enclave traffic flows through `tap`, `gvproxy`, ingress, and IMDS/AWS egress paths:  
  [docs/networking.md](docs/networking.md)
- **Usage** — CLI commands, configuration (`nitrum.toml`), and workflows for local and cloud deployments:  
  [docs/usage.md](docs/usage.md)
- **Releases** — SemVer, CHANGELOG, CI gates, and `nitrum.toml` compatibility:  
  [docs/releases.md](docs/releases.md)
- **Contributing** — development environment, style, and CI details:  
  [CONTRIBUTING.md](CONTRIBUTING.md)

### Samples and libraries

- **Hello sample** — minimal end‑to‑end project using the `nitrum` CLI: see `samples/hello` (and its `README.md`).
- **Blockchain wallet example** — see `samples/wallet` for a minimal secure wallet app scaffolded using Nitrum and running fully inside a Nitro Enclave.
- **TypeScript verifier** — `packages/nitrum-node` provides a Node.js helper for verifying Nitro Enclave attestation documents.

### License

Nitrum is released under the **MIT** license. See `LICENSE` for details.

<!--
TODO:

- [ ] Egress whitelist
- [x] Logs, filter sensitive values, headers, etc? (see `crates/observability`)
- [ ] Cloudformation template consolidation and export / import
-->
