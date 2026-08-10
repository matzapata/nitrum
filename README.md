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

### Examples and libraries

- **`nitrum init` template** — customer scaffold (git `nitrum-sdk`, project-dir Docker): see `crates/cli/template`.
- **Hello example** — in-repo Rust demo using path `crates/sdk`: see `examples/hello`.
- **Blockchain wallet example** — in-repo Rust enclave with JS integration tests: see `examples/wallet`.
- **`nitrum-node`** (`packages/node`) — Node.js napi bindings over `crates/verify` (document verify + TLS leaf hash bind).
- **`crates/sdk`** (`nitrum-sdk`) — In-enclave HTTP client for the data-plane crypto API (`:3000`).
- **`crates/verify`** — Pure Rust attestation verification library.

### Performance

Macro load on a real Nitro enclave (`m6i.xlarge`, 2 enclave vCPU / 4320 MiB, same-VPC k6 client, 50 VUs × 30s):

| Route | RPS | p50 | p99 | Errors |
|-------|-----|-----|-----|--------|
| `GET /.well-known/enclave/status` | ~10.2k | ~3.8 ms | ~20 ms | 0% |
| `GET /health` (proxied app) | ~10.5k | ~3.9 ms | ~17 ms | 0% |
| `POST /crypto` (proxy + encrypt/decrypt) | ~5.4k | ~8.4 ms | ~22 ms | 0% |

VU sweeps (10→200 VUs × 30s): status and health peak around **~11–12k RPS** (@ 100–200 VUs); crypto peaks near **~6.0k RPS** @ 200 VUs. Extra concurrency mostly raises latency rather than RPS. Proxied health tracks in-enclave status; crypto stays lower due to encrypt/decrypt work on the loopback crypto API.

Load harness: see [CONTRIBUTING.md](CONTRIBUTING.md#macro-load-testing-ec2-nitro).

### License

Nitrum is released under the **MIT** license. See `LICENSE` for details.

