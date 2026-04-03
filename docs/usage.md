# Usage

This guide explains how to **use Nitrum in practice**: what you need installed, how to scaffold a project, how to build and deploy enclaves, how secrets and logs work, and how to keep builds reproducible.

## Prerequisites

| Task | Requirements |
|------|----------------|
| Build CLI from source | [Rust](https://rustup.rs/) (stable) |
| `nitrum build`, `nitrum describe` | [Docker](https://docs.docker.com/get-docker/) |
| `nitrum local` | Docker with Compose support |
| `nitrum cloud deploy` / `nitrum cloud destroy` | [AWS credentials](https://docs.aws.amazon.com/cli/latest/userguide/cli-chap-configure.html) and permissions for CloudFormation, S3, KMS, and related resources |
| `nitrum cloud env` | AWS credentials with **SSM** `PutParameter` / `GetParameter` / `DeleteParameter` on `/nitrum/{name}/env/*` (`name` = `project.name` in `nitrum.toml`, same as CloudFormation **`ProjectName`**) |
| `nitrum cloud logs` | AWS credentials with **CloudWatch Logs** read access for `/nitrum/{name}/…` log groups |

Install the CLI from the repository root:

```bash
cargo install --path crates/cli
```

The installed binary is **`nitrum`**. The Cargo package name remains **`cli`** for the workspace; build it with `cargo build -p cli` or run without installing via `cargo run -p cli -- <args>` from the repo root.

## Project layout and example

After `nitrum init <name>`, a typical project contains:

- **`nitrum.toml`** — `project`, `runtime` images, service port, scaling, TLS, egress, and other settings.
- **`src/`** — your application (for example Node.js).
- **`Dockerfile`** — image used inside the enclave / dev stack.
- **`.nitrum/`** (created when needed) — generated Compose, CloudFormation template copies, and similar artifacts.

The repository includes a reference project under **`samples/hello`** which shows the end‑to‑end flow:

- Build the EIF: `nitrum build`.
- Set a simple environment variable: `nitrum cloud env set DEMO hello`.
- Deploy the enclave: `nitrum cloud deploy --eif .nitrum/artifacts/nitrum-hello.eif`.

Use that sample as a concrete reference when wiring your own projects.

### Built-in crypto and randomness endpoints

When you run `nitrum local up` or deploy with `nitrum cloud deploy`, the Nitrum **control-plane** exposes helper endpoints that your enclave code can call over the local bridge:

- **`POST http://localhost:3000/encrypt`** — body `{ "plaintext": "<string>" }`, returns an opaque JSON object you can later pass to `/decrypt`.
- **`POST http://localhost:3000/decrypt`** — body = the JSON you received from `/encrypt`, returns `{ "plaintext": "<original string>" }`.
- **`POST http://localhost:3000/random`** — body `{ "length": <number> }`, returns `{ "bytes": "<base64>" }` with cryptographically secure random bytes.

The `samples/hello/enclave/src/main.js` file demonstrates these patterns:

- Encrypt/decrypt round‑trip:

```js
const body = req.body && req.body.plaintext != null ? req.body : { plaintext: "" };
const { data: encrypted } = await axios.post("http://localhost:3000/encrypt", body, {
  headers: { "Content-Type": "application/json" },
});
const { data: decrypted } = await axios.post("http://localhost:3000/decrypt", encrypted, {
  headers: { "Content-Type": "application/json" },
});
```

- Random‑bytes generation:

```js
const { data } = await axios.post("http://localhost:3000/random", req.body, {
  headers: { "Content-Type": "application/json" },
});
```

The `samples/blockchain-wallet/enclave/src/main.js` sample builds on the same primitives to encrypt a wallet key and use it for signing without ever exposing the raw private key to the client.

## Commands

### `nitrum init [NAME]`

Scaffold a new project with a sample app and default `nitrum.toml`.

Typical first steps:

```bash
nitrum init my-app
cd my-app
nitrum build
```

### `nitrum build`

Builds the enclave Docker image and produces **`.nitrum/artifacts/{name}.eif`** in the project directory using **nitro-cli** inside Docker. Requires a valid `nitrum.toml` and project layout.

This command is the core of **reproducible builds**; see the dedicated section below for how to pin Docker inputs so the same source always yields the same EIF.

### `nitrum local`

Local development via Docker Compose:

- **`nitrum local up`** — start the stack in the background.
- **`nitrum local down`** — stop and remove containers.
- **`nitrum local logs`** — follow service logs.

Use this while iterating on your application code before pushing a new EIF to AWS.

### `nitrum cloud deploy`

Uploads the EIF (built from source if omitted) and creates or updates the **CloudFormation** stack using the bundled template and your `nitrum.toml`. Useful flags (see `nitrum cloud deploy --help`):

- **`--eif`** — path to an existing EIF file.
- **`--retain`** — retain selected resources on stack delete.
- **`--kms-administrator-role-arn`** — optional full IAM role or user ARN passed through as CloudFormation **`KmsAdministratorRoleArn`**. If you omit the flag, that parameter is not supplied and the template default applies. To look up your **account ID** when building ARNs, run: `aws sts get-caller-identity | jq -r '.Account'`.

The **`runtime.control_plane`** field in `nitrum.toml` is the full Docker image reference (for example `my-registry/nitrum-control-plane@sha256:…`) passed to CloudFormation for the EC2 control-plane service.

### `nitrum cloud destroy`

Tears down deployed AWS resources (see command help for options such as retaining KMS/SSM data). Use this to clean up trial stacks or rotate between environments.

### `nitrum cloud env`

Manage **application** environment variables as **SSM Parameter Store** `SecureString` values under `/nitrum/{name}/env/{KEY}`, where **`name`** is **`project.name`** in `nitrum.toml` (same value as the CloudFormation stack name and **`ProjectName`** parameter).

- **`nitrum cloud env set KEY VALUE`** — create or overwrite a parameter.
- **`nitrum cloud env get`** — list every app env parameter as `KEY=value` (decrypted; sensitive).
- **`nitrum cloud env delete KEY`** — remove the parameter.

At runtime, the **data-plane** loads every parameter under that path at startup (unless `NITRUM_APP_ENV_SSM_PREFIX` is set to empty to skip) and passes them to the **user process environment**, overlaying the parent environment.

### `nitrum cloud logs`

Stream or poll **CloudWatch Logs** for the deployed data-plane or control-plane (`--service data-plane` or `control-plane`).

This is the main way to debug enclaves in the field: look for TLS/ACME, KMS, or app‑level errors in these streams.

### `nitrum describe`

Runs **`nitro-cli describe-eif`** in Docker against an EIF path (wrapper for inspecting measurements and metadata). Use this to pull out PCRs you want to enforce from your verification or KMS policies.

## `nitrum.toml` overview

Options are defined in the `shared` crate; the sample project comments point to the source. Common sections:

- **`[project]` `name`** — project identifier; CloudFormation stack name and **`ProjectName`** match it; S3 bucket is **`nitrum-{name}`**; SSM paths use **`/nitrum/{name}/…`** (data-plane infra and app env).
- **`[runtime]` `data_plane`** — Docker image passed as `DATA_PLANE_IMAGE` / Dockerfile `ARG` for **`nitrum build`** and **`nitrum local`** (base containing the in-enclave data-plane).
- **`[runtime]` `control_plane`** — full image ref for the host control-plane on **`nitrum cloud deploy`** (CloudFormation).
- **`[runtime]` `nitro_cli`** — image for **`nitro-cli`** (EIF build and **`nitrum describe`**).
- **`[service]`** — listen port for your app.
- **`[health_check]`** — path, port, and interval for health checks.
- **`[scaling]`** — replica hints and enclave CPU/RAM (used in deployment templates).
- **`[tls_termination]`** — `acme` and `domain` for certificates.
- **`[egress]`** — `enabled` and `destinations` for outbound restrictions (see code and templates for current behavior).

Edit `nitrum.toml` to match your app’s port, domain, and infrastructure expectations, then rebuild the EIF and redeploy when you change enclave-related settings.

## Reproducible builds

Because attestation policies and KMS recipient conditions are typically pinned to **specific EIF measurements**, it is important that a given source tree and configuration always yields the **same EIF hash**. Nitrum leans on Docker for this, so you should:

- **Pin all base images by digest** in your `Dockerfile`:

```Dockerfile
FROM node:20-alpine@sha256:<exact_digest>
```

- **Avoid floating tags** in `nitrum.toml` runtime images:
  - Use `runtime.data_plane = "ghcr.io/…/nitrum-data-plane@sha256:…"` instead of `:latest`.
  - Use `runtime.nitro_cli = "…@sha256:…"` for the nitro-cli image.
- **Keep build inputs declarative**:
  - Avoid downloading tools or dependencies at build time without pinning versions and checksums.
  - Prefer lockfiles (`package-lock.json`, `Cargo.lock`) and checksum‑verified downloads when possible.

With these constraints in place:

1. `nitrum build` will turn your Docker image into an EIF via `nitro-cli` in a fully specified environment.
2. Running the same build on another machine (or in CI) should produce an EIF with the **same measurements**.
3. You can safely copy PCRs from `nitrum describe` into:
   - Client‑side verification code (for example using `nitrum-node`).
   - KMS key policies that require specific PCRs before decrypting.

## Documentation map

- [architecture.md](architecture.md) — how control-plane, data-plane, and AWS pieces fit together.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — developing and testing the Rust workspace.
