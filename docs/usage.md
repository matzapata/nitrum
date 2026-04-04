# Usage

This guide explains how to use Nitrum in practice: what you need installed, how to scaffold a project, how to build and deploy enclaves, how secrets and logs work, and how to keep builds reproducible.

## Requirements

### Docker

- **Docker Engine** with the daemon running (`docker info` should succeed).
- **Buildx / BuildKit**: `nitrum build` invokes `docker build --platform linux/amd64`. Docker Desktop and current Docker CE installs include this; on Linux, install the **docker-buildx** plugin if `docker buildx version` is missing.
- **Docker Compose v2** (`docker compose`): `nitrum local` writes a Compose file under `.nitrum/` and expects the v2 CLI (not the legacy `docker-compose` Python binary).
- **Platform**: Enclave images and `nitro-cli` run as **linux/amd64**. On **Apple Silicon** or other non-amd64 hosts, Docker uses QEMU/binfmt emulation; first builds may be slow until base layers are cached.
- **Resources**: EIF builds pull several images (`DATA_PLANE_IMAGE`, `nitro-cli`, your app base images). Ensure enough free disk space and registry access for everything referenced in your `Dockerfile` and `nitrum.toml`.

### AWS CLI

Needed only if you use `**nitrum cloud`** (deploy, env, logs, destroy). Local-only workflows (`nitrum build`, `nitrum local`) do not require it.

## Project layout and example

`nitrum init [name]` by default creates a sample Node.js server, but you can use any language or stack as long as the project includes a **Dockerfile** that exposes the **application port** from `[service]` in `nitrum.toml`.

The repository includes a reference project under `samples/hello` which shows the end‑to‑end flow:

- Build the EIF: `nitrum build`.
- Set a simple environment variable: `nitrum cloud env set DEMO hello`.
- Deploy the enclave: `nitrum cloud deploy`

Use that sample as a concrete reference when wiring your own projects.

### Ingress HTTPS API (external, `/.well-known/...`)

The in-enclave **data-plane** terminates **TLS** on `NITRUM_INGRESS_LISTEN_ADDR` (default `**0.0.0.0:443`**). Clients reach these URLs over **HTTPS** (for example `https://nitrum.local` in the local Compose stack, or your deployed domain). Requests on the paths below are handled **inside the ingress**; everything else is **reverse-proxied** over HTTP to your app at the port from `[service]` in `nitrum.toml`.

#### Endpoints reachable from the Internet (or local TLS client)

- `GET /.well-known/enclave/status` Returns a minimal liveness payload for the ingress/data-plane.  
If all goes well, the enclave responds with status code `200 OK` and a JSON body:
  ```
  {
    "status": "ok"
  }
  ```
- `GET /.well-known/enclave/attestation` Returns an **AWS Nitro attestation document** for the running enclave, with the **current TLS leaf certificate** bound into the NSM request (certificate hash as `public_key` material).  
Optional query parameter: `nonce` — standard Base64 encoding of raw nonce bytes.  
If all goes well, the enclave responds with status code `200 OK` and:
  ```
  {
    "data": "<base64-encoded attestation document (raw COSE/CBOR bytes)>"
  }
  ```
  If the TLS certificate is not yet available, the enclave responds with status code `503 Service Unavailable` and a JSON body such as `{"error":"TLS certificate not yet available"}`. Attestation failures may yield `500 Internal Server Error` with `{"error":"attestation failed: ..."}`.
- `GET /.well-known/acme-challenge/{token}` Served only when `**[tls_termination].acme**` is enabled. A separate **plain-HTTP** listener on `NITRUM_ACME_HTTP01_LISTEN_ADDR` (default `**0.0.0.0:80`**) exposes this path for **ACME HTTP-01** validation. If all goes well, the server responds with status code `200 OK` and the challenge key authorization bytes as the body (content type `application/octet-stream`).

### Data-plane crypto HTTP API (internal)

When you run `nitrum local up` or deploy with `nitrum cloud deploy`, the in-enclave data-plane serves a small JSON HTTP API for encryption and randomness on `NITRUM_CRYPTO_API_LISTEN_ADDR` (default `**0.0.0.0:3000`**). Application code inside the enclave typically calls `http://localhost:3000/...`.

#### Endpoints reachable from application code

- `POST /encrypt` Encrypts a UTF-8 plaintext string.  
The request must use `Content-Type: application/json` with a body of the form:
  ```
  {
    "plaintext": "<string>"
  }
  ```
  If all goes well, the server responds with status code `200 OK` and a JSON body:
  ```
  {
    "data": "<base64-encoded ciphertext>",
    "error": null
  }
  ```
  Treat `data` as opaque; supply it as `ciphertext` to `/decrypt` (see below). On failure, `data` is `null` and `error` carries a message (for example `500 Internal Server Error` when encryption fails).
- `POST /decrypt` Decrypts ciphertext produced by `/encrypt`.  
The request must use `Content-Type: application/json` with a body of the form:
  ```
  {
    "ciphertext": "<base64 string, same as encrypt response data>"
  }
  ```
  If all goes well, the server responds with status code `200 OK` and:
  ```
  {
    "data": "<original plaintext string>",
    "error": null
  }
  ```
  Invalid Base64 may yield `400 Bad Request`; decryption failures or decrypted bytes that are not valid UTF-8 may yield `422 Unprocessable Entity`, with `data` set to `null` and an `error` message.
- `POST /random` Returns cryptographically secure random bytes.  
The request may be empty or use `Content-Type: application/json` with an optional body:
  ```
  {
    "length": <number>
  }
  ```
  `length` defaults to `32` and is capped at `1024`. If all goes well, the server responds with status code `200 OK` and:
  ```
  {
    "data": "<base64-encoded bytes>",
    "error": null
  }
  ```

The `samples/hello/enclave/src/main.js` file demonstrates an encrypt/decrypt round-trip and `/random` over HTTP. The `samples/wallet/enclave/src/main.js` sample builds on the same primitives to encrypt a wallet key and use it for signing without ever exposing the raw private key to the client.

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

Builds the enclave Docker image and produces `.nitrum/artifacts/{name}.eif` in the project directory using nitro-cli inside Docker. Requires a valid `nitrum.toml` and project layout.

This command is the core of reproducible builds; see the dedicated section below for how to pin Docker inputs so the same source always yields the same EIF.

### `nitrum local`

Local development via Docker Compose:

- `nitrum local up` — start the stack in the background.
- `nitrum local down` — stop and remove containers.
- `nitrum local logs` — follow service logs.

Use this while iterating on your application code before pushing a new EIF to AWS.

### `nitrum cloud deploy`

Uploads the EIF (built from source if omitted) and creates or updates the CloudFormation stack using the bundled template and your `nitrum.toml`. Useful flags (see `nitrum cloud deploy --help`):

- `--eif` — path to an existing EIF file.
- `--retain` — retain resources on stack delete (set this to true for production deployments).
- `--kms-administrator-role-arn` — optional full IAM role or user ARN passed through as CloudFormation `KmsAdministratorRoleArn`. If you omit the flag, that parameter is not supplied and the template default applies. To look up your account ID when building ARNs, run: `aws sts get-caller-identity | jq -r '.Account'`.

The `runtime.control_plane` field in `nitrum.toml` is the full Docker image reference (for example `my-registry/control-plane@sha256:…`) passed to CloudFormation for the EC2 control-plane service.

### `nitrum cloud destroy`

Tears down deployed AWS resources (see command help for options such as retaining KMS/SSM data). Use this to clean up trial stacks or rotate between environments.

### `nitrum cloud env`

Manage application environment variables as SSM Parameter Store `SecureString` values under `/nitrum/{name}/env/{KEY}`, where `name` is `project.name` in `nitrum.toml` (same value as the CloudFormation stack name and `ProjectName` parameter).

- `nitrum cloud env set KEY VALUE` — create or overwrite a parameter.
- `nitrum cloud env get` — list every app env parameter as `KEY=value` (decrypted; sensitive).
- `nitrum cloud env delete KEY` — remove the parameter.

At runtime, the data-plane loads every parameter under that path at startup (unless `NITRUM_APP_ENV_SSM_PREFIX` is set to empty to skip) and passes them to the user process environment, overlaying the parent environment.

### `nitrum cloud logs`

Stream or poll CloudWatch Logs for the deployed data-plane or control-plane (`--service data-plane` or `control-plane`).

This is the main way to debug enclaves in the field: look for TLS/ACME, KMS, or app‑level errors in these streams.

### `nitrum describe`

Runs `nitro-cli describe-eif` in Docker against an EIF path (wrapper for inspecting measurements and metadata). Use this to pull out PCRs you want to enforce from your verification or KMS policies.

## `nitrum.toml` overview

Options are defined in the `shared` crate; the sample project comments point to the source. Common sections:

- `[project]` `name` — project identifier; CloudFormation stack name and `ProjectName` match it; S3 bucket is `nitrum-{name}`; SSM paths use `/nitrum/{name}/…` (data-plane infra and app env).
- `[runtime]` `data_plane` — Docker image passed as `DATA_PLANE_IMAGE` / Dockerfile `ARG` for `nitrum build` and `nitrum local` (base containing the in-enclave data-plane).
- `[runtime]` `control_plane` — full image ref for the host control-plane on `nitrum cloud deploy` (CloudFormation).
- `[runtime]` `nitro_cli` — image for `nitro-cli` (EIF build and `nitrum describe`).
- `[service]` — listen port for your app.
- `[health_check]` — path, port, and interval for health checks.
- `[scaling]` — replica hints and enclave CPU/RAM (used in deployment templates).
- `[tls_termination]` — `acme` and `domain` for certificates.
- `[egress]` — `enabled` and `destinations` for outbound restrictions (see code and templates for current behavior).

Edit `nitrum.toml` to match your app’s port, domain, and infrastructure expectations, then rebuild the EIF and redeploy when you change enclave-related settings.

## Reproducible builds

Because user trust attestation policies and KMS recipient conditions are typically pinned to specific EIF measurements, it is important that a given source tree and configuration always yields the same EIF hash. Nitrum leans on Docker for this, so you should:

- Pin all base images by digest in your `Dockerfile`:

```Dockerfile
FROM node:20-alpine@sha256:<exact_digest>
```

- Avoid floating tags in `nitrum.toml` runtime images:
  - **`nitrum init` does this for you by default** for the bundled Nitrum runtime images: it resolves `data_plane`, `control_plane`, and `nitro_cli` from their template `:latest` tags to immutable `@sha256:…` references and writes those into `nitrum.toml`. Feel free to update them manually to whatever you want.
- Keep build inputs declarative:
  - Avoid downloading tools or dependencies at build time without pinning versions and checksums.
  - Prefer lockfiles (`package-lock.json`, `Cargo.lock`) and checksum‑verified downloads when possible.

With these constraints in place:

1. `nitrum build` will turn your Docker image into an EIF via `nitro-cli` in a fully specified environment.
2. Running the same build on another machine (or in CI) should produce an EIF with the same measurements.
3. You can safely copy PCRs from `nitrum describe` into:
  - Client‑side verification code (for example using `nitrum-node`).
  - KMS key policies that require specific PCRs before decrypting.

## Workflow

Typical path from a new project to a deployed enclave:

1. **Scaffold** — `nitrum init my-app`, then `cd my-app` and adjust `nitrum.toml` and your app as needed.
2. **Iterate locally** — `nitrum local up` to run the Compose stack; use `nitrum local logs` to debug; `nitrum local down` when you are done.
3. **Build** — `nitrum build` to produce the EIF under `.nitrum/artifacts/`.
4. **Configure secrets and env** — `nitrum cloud env set KEY VALUE` (and related `env` subcommands) so the enclave has the parameters it needs at startup. Requires AWS credentials configured for `nitrum cloud`.
5. **Deploy** — `nitrum cloud deploy` to create or update the stack from `nitrum.toml` and the built EIF.
6. **DNS** — Point your `[tls_termination].domain` at the load balancer. 

After deploy, use `nitrum cloud logs` for production debugging and `nitrum describe` when you need EIF measurements for verification or policy.

## Documentation map

- [architecture.md](architecture.md) — how control-plane, data-plane, and AWS pieces fit together.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — developing and testing the Rust workspace.

