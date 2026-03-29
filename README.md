# Nitrum

Nitrum is a **Rust** toolkit for running applications in **AWS Nitro Enclaves**. It pairs an in-enclave **data-plane** with a parent **control-plane** and a **`nitrum` CLI** that scaffolds projects, builds **EIF** images, runs a local Docker stack, and deploys with **CloudFormation**.

In spirit, it belongs to the same family as **[nitriding](https://github.com/brave/nitriding)** (Go): **TLS and attestation are handled inside the enclave**, your app stays simple (often plain HTTP on localhost), and **gvproxy** provides **TAP-style networking** so you are not wiring **vsock** yourself. Nitrum is a **different codebase and API surface**—implemented here in Rust with `nitrum.toml`, storage-backed ACME, and KMS patterns documented in [docs/architecture.md](docs/architecture.md).

## Enclave runtime (data-plane)

These features run **inside** the Nitro Enclave:

- **TLS termination in the enclave** — Clients connect with **HTTPS**. The data-plane terminates TLS with **rustls** and forwards requests to your process as **HTTP** on `127.0.0.1` and the port from **`nitrum.toml`** (`ingress_proxy`). The EC2 host and **gvproxy** forward bytes; they do **not** decrypt application TLS.

- **Certificates without app code changes** — The data-plane starts from an ephemeral **self-signed** certificate, records a **hash** for attestation binding, and can optionally use **ACME HTTP-01** (e.g. **Let’s Encrypt**, or **Pebble** in local dev). Issued **certificate and key** are persisted via the configured **storage** backend so renewals can **reload TLS** without restarting the whole enclave.

- **Remote attestation endpoint** — **`GET /.well-known/enclave/attestation`** returns a **Nitro NSM** attestation document (JSON with base64 payload). The request incorporates the **TLS certificate material hash** so verifiers can relate **what they see on the wire** to **measured boot / KMS-style policies**. **`GET /.well-known/enclave/status`** exposes a small JSON health response.

- **Egress and AWS from the enclave** — On the host, **gvproxy** (`gvisor-tap-vsock`) bridges **vsock** and a **TAP** path into the enclave so outbound TCP (HTTPS to Let’s Encrypt, AWS APIs, etc.) works without custom tunnel code. With **`-ec2-metadata-access`**, the data-plane can use **IMDSv2** at **`169.254.169.254`** for **instance role credentials** and then call **KMS**, **S3**, **DynamoDB**, and other services. **`[egress]`** in `nitrum.toml` describes outbound policy; **strict allowlist enforcement** is still on the [roadmap](#status) (config is there; end-to-end enforcement is evolving).

- **Crypto helpers** — **`crypto/kms.rs`** supports envelopes such as **`GenerateDataKeyWithoutPlaintext`** and **attested `Decrypt`** in enclave builds, aligned with AWS Nitro **recipient attestation** expectations.

**Not the same as nitriding:** Nitrum does **not** mirror nitriding’s Go HTTP API (`/enclave/attestation` path shape, horizontal-scaling registration endpoints, or optional “register hash over app public key” flows). If you need those exact primitives, compare both projects and [docs/architecture.md](docs/architecture.md).

## Host runtime (control-plane)

On the **parent EC2 instance**, the **control-plane** starts **gvproxy** (listen addresses, port forwards, **EC2 metadata** bridging) and drives **nitro-cli** to **run** and monitor the enclave **EIF**. It **does not** terminate your application’s TLS.

## CLI and workflow

- **`nitrum init`** — Sample app, `Dockerfile`, and `nitrum.toml`.
- **`nitrum build`** — Enclave image and **`enclave.eif`** via Docker and **nitro-cli**.
- **`nitrum dev`** — Local **Docker Compose** stack.
- **`nitrum deploy` / `nitrum destroy`** — **S3** EIF artifact and **CloudFormation** stack.
- **`nitrum describe`** — **EIF** measurements/metadata via **nitro-cli** in Docker.

Install from the repo: `cargo install --path crates/cli` (binary **`nitrum`**; Cargo package name is still **`cli`** — use `cargo run -p cli -- …` without installing). Command reference: [docs/usage.md](docs/usage.md).

## Repository layout

| Path | Description |
|------|-------------|
| `crates/cli` | `nitrum` command-line tool |
| `crates/control-plane` | Host-side **gvproxy** and **nitro-cli** enclave lifecycle |
| `crates/data-plane` | In-enclave TLS ingress, ACME, attestation, proxy to your app, IMDS/KMS integration |
| `crates/shared` | Shared config types for `nitrum.toml` |
| `samples/hello` | Reference project |
| `infra/docker` | Dockerfiles for control-plane, data-plane, and nitro-cli images |

## Docs

- **[docs/architecture.md](docs/architecture.md)** — Control-plane vs data-plane, TLS/ACME/attestation/IMDS sequences (Mermaid).
- **[docs/usage.md](docs/usage.md)** — CLI reference, prerequisites, `nitrum.toml`.
- **[CONTRIBUTING.md](CONTRIBUTING.md)** — Build, test, and contribute.
- **[LICENSE](LICENSE)** — MIT.

## Quick start (from source)

```bash
cargo install --path crates/cli
nitrum init my-app
cd my-app
nitrum dev up
# nitrum build && nitrum deploy   # AWS: credentials + EIF when ready
```

## Status

Nitrum is **early-stage**. APIs, defaults, and CloudFormation resources may change—review templates and `nitrum.toml` before production accounts.


<!-- 
TODO: docs with diagrams like https://github.com/aws-samples/custom-attestation-multi-party-crypto-wallet-with-aws-nitro-enclave/blob/main/README.md
- [ ] Control plane resilience: watchdog, restart if killed, clearer logs, health endpoints
- [ ] Exercise TLS certificates with ACME in real deployments
- [ ] test cert in attestation
- [ ] Verify certs are stored encrypted at rest
- [ ] Networking cleanup
- [ ] Enclave / control-plane logs to CloudWatch
- [ ] Instances config, ram, etc, look at config
- [ ] CloudFormation: PCR0 / KMS policy alignment documented and verified
- [ ] Env vars support (SSM params, rollout depl)
- [ ] Js sdk?
- [ ] Reproducible builds
- [ ] Add samples for MCP / MPC / etc
- [ ] Egress allowlist enforced end-to-end (`[egress].destinations` in config today)
-->