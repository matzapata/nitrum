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
- **`nitrum build`** — Enclave image and **`.nitrum/artifacts/{name}.eif`** via Docker and **nitro-cli**.
- **`nitrum local`** — Local **Docker Compose** stack (`up`, `down`, `logs`).
- **`nitrum cloud deploy` / `nitrum cloud destroy`** — **S3** EIF artifact and **CloudFormation** stack.
- **`nitrum cloud env set|get|delete`** — Application secrets in **SSM** (`SecureString` under `/nitrum/{name}/env/` for `nitrum.toml` **`project.name`**); the **data-plane** loads them at startup and injects them into the **user process** environment (overlay on the parent env).
- **`nitrum cloud logs`** — **CloudWatch** logs for deployed control-plane and data-plane.
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
nitrum local up
# nitrum build && nitrum cloud deploy   # AWS: credentials + EIF when ready
```

## Status

Nitrum is **early-stage**. APIs, defaults, and CloudFormation resources may change—review templates and `nitrum.toml` before production accounts.


<!-- 
Docs
- [ ] Reproducible builds (add docs on this, it's achieved by fixing with sha the artifacts images and then user docker image)
- [] TODO: docs with diagrams like https://github.com/aws-samples/custom-attestation-multi-party-crypto-wallet-with-aws-nitro-enclave/blob/main/README.md
- [X] Js sample
- [ ] Add samples. Some ideas: ML inference, smth like: https://github.com/aws-samples/aws-nitro-enclaves-llm/blob/main/src/enclave/server.py Crypto wallet like https://github.com/aws-samples/aws-nitro-enclave-blockchain-wallet/blob/main/application/eth1/lambda/lambda_function.py or better like https://github.com/aws-samples/nitro-enclave-blockchain-wallet-on-eks/tree/main 
- [] Domain config instructions

Testing
- [ ] Some automated tests
- [ ] TODO: add but after testing the rest. CloudFormation: PCR0 / KMS policy alignment documented and verified
- [X] Exercise TLS certificates with ACME in real deployments
- [ ] test cert in attestation
- [ ] Verify certs are stored encrypted at rest


Release
- [ ] Pipeline
- [ ] tags, images, etc
- [ ] cli install script


Attestation:
{"document":"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"}

-->