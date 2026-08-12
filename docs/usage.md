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

`nitrum init [name]` scaffolds from [`crates/cli/template`](../crates/cli/template) (git `nitrum-sdk` dependency, project-directory Docker build). You can use any language or stack as long as the project includes a **Dockerfile** that exposes the **application port** from `[project].port` in `nitrum.toml` and runs the data-plane with your bundled `nitrum.toml` (for example `CMD ["/app/data-plane", "--config", "/app/nitrum.toml"]`). The data-plane reads `[project].start_command` from that file and starts the user process.

In-repo demos under `examples/hello` and `examples/wallet` use the same project-directory Docker layout (git `nitrum-sdk` + workspace `[patch]` to local [`crates/sdk`](../crates/sdk)); they are not what `nitrum init` copies. Typical flow on a scaffolded or example project:

- Build the EIF: `nitrum build`.
- Set a simple environment variable: `nitrum cloud env set DEMO hello`.
- Deploy the enclave: `nitrum cloud deploy`


### Ingress HTTPS API (external, `/.well-known/...`)

The in-enclave **data-plane** terminates **TLS** on `NITRUM_INGRESS_LISTEN_ADDR` (default `**0.0.0.0:443`**). Clients reach these URLs over **HTTPS** (for example `https://nitrum.localhost` in the local Compose stack, or your deployed domain). Platform paths below are always handled **inside the ingress**; everything else is **reverse-proxied** over HTTP to your app at the port from `[project].port` in `nitrum.toml`.

#### Endpoints reachable from the Internet (or local TLS client)

- `GET /.well-known/enclave/status` — readiness for the hosted app. On success: `200 OK` and `{"status":"ok"}` when `[health_check]` probes succeed (or there is no `start_command`). On failure: `503` and `{"status":"unhealthy"}`. NLB HTTPS health checks use this path.

- `GET /.well-known/enclave/attestation` — **AWS Nitro attestation document** for the running enclave, with the **current TLS leaf certificate** bound into the NSM request (certificate hash as `public_key` material). Optional query: `nonce` (standard Base64 of raw nonce bytes). On success: `200 OK` and `{"data":"<base64-encoded attestation document>"}`. If the TLS certificate is not yet available: `503` with `{"error":"TLS certificate not yet available"}`. Attestation errors may return `500` with `{"error":"attestation failed: ..."}`.

When `[tls_termination].acme` is enabled:

- `GET /.well-known/acme-challenge/{token}` — **ACME HTTP-01** on a separate plain-HTTP listener (`NITRUM_ACME_HTTP01_LISTEN_ADDR`, default `0.0.0.0:80`). On success: `200 OK` and the challenge key authorization bytes (`application/octet-stream`).

### Data-plane crypto HTTP API (internal)

When you run `nitrum local up` or deploy with `nitrum cloud deploy`, the in-enclave data-plane serves a small JSON HTTP API for encryption, decryption, attestation, and randomness on `NITRUM_CRYPTO_API_LISTEN_ADDR` (default `**0.0.0.0:3000`**). Application code inside the enclave typically calls `http://localhost:3000/...`.

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

The `examples/hello` Rust app demonstrates an encrypt/decrypt round-trip and `/random`. The `examples/wallet` example uses the same crypto endpoints and returns sealed wallet ciphertext to the caller; see `examples/wallet/tests/integration.test.mjs` for the create-and-sign flow.

## Observability

Nitrum emits telemetry through a single path: OpenTelemetry. Both the control-plane (on the EC2 host) and the data-plane (inside the enclave) always write structured logs to stdout and, when an OTLP collector endpoint is configured, additionally export **traces, metrics, and logs** over OTLP/gRPC to a local OpenTelemetry Collector. The collector (the AWS Distro for OpenTelemetry, ADOT, on the EC2 host) translates OTLP into CloudWatch metrics (`Nitrum` namespace via EMF), CloudWatch Logs (`/nitrum/{project}/data-plane` and `/nitrum/{project}/control-plane`), and — when `cloud.xray_tracing = true` — X-Ray traces. The binaries still emit backend-neutral OTLP; the deployed data-plane only uses IMDS to discover the parent host address for its default collector endpoint.

### Platform vs application telemetry

Nitrum separates **platform** telemetry from **application** telemetry in OpenTelemetry:

| Layer | `service.name` | `nitrum.component` | Metric prefix (examples) |
|-------|----------------|--------------------|--------------------------|
| Control-plane | `control-plane` | `core` | `nitrum.enclave.restarts` |
| Data-plane | `data-plane` | `core` | `nitrum.requests`, `nitrum.acme.events` |
| Your app | `project.name` from `nitrum.toml` | `user-app` | your choice (examples use `app.*`) |

Platform binaries always set `service.namespace=nitrum`. When OTLP export is enabled, the data-plane also injects standard OpenTelemetry environment variables into your application process before it starts:

| Variable | Value |
|----------|--------|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Same collector endpoint as the data-plane |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | `grpc` |
| `OTEL_SERVICE_NAME` | `project.name` from `nitrum.toml` |
| `OTEL_RESOURCE_ATTRIBUTES` | `nitrum.component=user-app,service.namespace=nitrum` |

Your app uses the OpenTelemetry SDK for your language and reads those variables — no Nitrum-specific client is required. In `nitrum local`, open Grafana at `http://localhost:3000` and filter by `service.name` or `nitrum.component` to compare platform and app series. In the cloud, CloudWatch EMF uses `ServiceName` (from `service.name`) as the log stream name under `/nitrum/{project}/metrics`.

See `examples/hello/src/main.rs` for a minimal Rust example (`app.crypto.ops`).

### `NITRUM_OTLP_ENDPOINT`

Selects the OTLP/gRPC collector endpoint.

- **Unset** — use the platform default. On the deployed stack, after enclave networking is up,
  the data-plane reads the parent instance private IPv4 from IMDS (`meta-data/local-ipv4`) and
  exports to `http://{parent-private-ip}:4317`, where the ADOT collector is published.
- **Empty** — stdout-only. No collector is contacted.
- **Set** — also export OTLP to that endpoint. On the deployed stack:
  - control-plane: `http://127.0.0.1:4317` (collector shares the control-plane container's network namespace).
  - data-plane (enclave): `http://{parent-private-ip}:4317` by default. The parent private IPv4 comes from IMDS `meta-data/local-ipv4`, and CloudFormation publishes the ADOT collector on port `4317`. Override `NITRUM_OTLP_ENDPOINT` only when you want a different collector, or set it to an empty value for stdout-only.

`RUST_LOG` controls the log/trace filter (default `info`) for both stdout and OTLP.

When egress enforcement is enabled (`[egress].enabled`), the effective OTLP collector endpoint (the env override or the platform default) is automatically allowlisted so OTLP export is not dropped. IP endpoints are also excluded from the transparent proxy, mirroring the IMDS bypass; hostname endpoints use the normal DNS allowlist path.

### Redaction

Telemetry only ever carries low-cardinality, non-sensitive attributes: service name, matched **route template** (never the raw URL), HTTP method, status class, and operation names (e.g. KMS `decrypt`). Request and response **headers, bodies, query strings, and secrets are never recorded** in logs, spans, or metrics.

## Commands

### `nitrum init [NAME]`

Scaffold a new project from [`crates/cli/template`](../crates/cli/template): sample Rust app, `Dockerfile`, default `nitrum.toml`, and `tests/integration.test.mjs` (Node’s test runner + optional `nitrum-node` attestation checks when `ENCLAVE_URL` points at a real deployment).

Typical first steps:

```bash
nitrum init my-app
cd my-app
npm install
nitrum build
```

### `nitrum build`

Builds the enclave Docker image and produces `.nitrum/artifacts/{name}.eif` in the project directory using nitro-cli inside Docker. Requires a valid `nitrum.toml` and project layout.

This command is the core of reproducible builds; see the dedicated section below for how to pin Docker inputs so the same source always yields the same EIF.

### `nitrum local`

Local development via Docker Compose:

- `nitrum local up` — build the enclave image, then start the stack in the background. Applies `[scaling].num_cpus` and `ram_size_mib` as Compose CPU/memory limits on the `enclave` service (same budget cloud passes to `nitro-cli`).
- `nitrum local down` — stop and remove containers.
- `nitrum local logs` — follow service logs.

The enclave image is built by the CLI (not Compose) before `up`. The Dockerfile’s `DATA_PLANE_IMAGE` build arg comes from
`[runtime].data_plane` in `nitrum.toml` (overridable via
`NITRUM_RUNTIME_DATA_PLANE_IMAGE`), with a `-local` tag suffix applied
automatically so the pebble-enabled image is used
(e.g. `…/data-plane:latest` → `…/data-plane:latest-local`). Digest pins from
`nitrum init` map to `:latest-local` on the same repository. See
[CONTRIBUTING.md](../CONTRIBUTING.md#building-platform-images-for-development)
for `docker buildx bake` snippets.

Use this while iterating on your application code before pushing a new EIF to AWS.

### `nitrum cloud deploy`

Uploads the EIF (built from source if omitted) and creates or updates the CloudFormation stack using the bundled template and your `nitrum.toml`. Useful flags (see `nitrum cloud deploy --help`):

- `--eif` — path to an existing EIF file.
- `--retain` — retain resources on stack delete (set this for production). Also enables DynamoDB point-in-time recovery and deletion protection.
- `--debug-mode` — pass `--debug-mode` to the control-plane (and use an all-zero PCR0 in the KMS policy for that deploy).
- `--force` — skip the confirmation prompt.

CloudFormation knobs that used to be one-shot CLI flags now live under **`[cloud]`** in `nitrum.toml` (see below) so every deploy re-supplies them. In particular, **`cloud.kms_administrator_role_arn`** replaced `--kms-administrator-role-arn`: omitting that flag on a later update used to reset the KMS admin principal to account root.

The `runtime.control_plane` field in `nitrum.toml` is the full Docker image reference (for example `my-registry/control-plane@sha256:…`) passed to CloudFormation for the EC2 control-plane service.

#### Updating a deployment

Each `nitrum cloud deploy` with a new EIF:

1. Uploads `{sha12}.eif` and sets `EifVersionLabel` / `EifS3Key` so the launch template changes and the **ASG rolls** instances (with `cloud.safe_rolling`, at least one instance stays in service and the update pauses ~5 minutes for enclave boot).
2. Sets `EifImageSha384` to the new EIF’s **PCR0**, which updates the KMS key policy’s `kms:RecipientAttestation:ImageSha384` condition **in place** (same key id).
3. New hosts download the new EIF; attested `Decrypt` succeeds only for the new PCR0. During the roll, old enclaves may briefly fail attested Decrypt after the policy swaps.

Only the KMS **administrator** principal (`cloud.kms_administrator_role_arn`, or account root when empty) can change that PCR0 condition (`kms:PutKeyPolicy`). The identity running `nitrum cloud deploy` must match that principal (or assume that role); otherwise the stack update fails with `AccessDenied`. The CLI warns when `sts:GetCallerIdentity` does not match a configured admin ARN.

#### Production checklist

- Deploy with `--retain`.
- Set `scaling.max_replicas >= desired_replicas + 1` and keep `cloud.safe_rolling = true`.
- Pin `cloud.kms_administrator_role_arn` to your deployer/admin role (and deploy *as* that role).
- Set `cloud.sns_alarm_topic_arn` to an existing SNS topic for NLB unhealthy-host and ASG capacity alarms.
- Enable `cloud.xray_tracing = true` only if you want X-Ray (CloudWatch logs/metrics still work when it is false).
- Point DNS for `[tls_termination].domain` at the NLB.

### `nitrum cloud destroy`

Tears down deployed AWS resources (see command help for options such as retaining KMS/SSM data). Use this to clean up trial stacks or rotate between environments.

### `nitrum cloud env`

Manage application environment variables as SSM Parameter Store `SecureString` values under `/nitrum/{name}/env/{KEY}`, where `name` is `project.name` in `nitrum.toml` (same value as the CloudFormation stack name and `ProjectName` parameter).

- `nitrum cloud env set KEY VALUE` — create or overwrite a parameter.
- `nitrum cloud env get` — list every app env parameter as `KEY=value` (decrypted; sensitive).
- `nitrum cloud env delete KEY` — remove the parameter (add `--force` to skip the confirmation prompt, for scripts).

At runtime, the data-plane loads every parameter under that path at startup (unless `NITRUM_APP_ENV_SSM_PREFIX` is set to empty to skip) and passes them to the user process environment, overlaying the parent environment.

### `nitrum cloud logs`

Stream or poll CloudWatch Logs for the deployed data-plane or control-plane (`--service data-plane` or `control-plane`).

This is the main way to debug enclaves in the field: look for TLS/ACME, KMS, or app‑level errors in these streams.

### `nitrum describe`

Runs `nitro-cli describe-eif` in Docker against an EIF path (wrapper for inspecting measurements and metadata). Use this to pull out PCRs you want to enforce from your verification or KMS policies.

## `nitrum.toml` overview

Options are defined in the `config` crate; the sample project comments point to the source. Common sections:

- `[project]` `name` — project identifier; CloudFormation stack name and `ProjectName` match it; S3 bucket is `nitrum-{name}`; SSM paths use `/nitrum/{name}/…` (data-plane infra and app env).
- `[project]` `port` — TCP port your app listens on at `127.0.0.1` (ingress proxies here after TLS).
- `[project]` `start_command` — argv for the user process (JSON array in `nitrum.toml`); the data-plane spawns it after loading config.
- `[runtime]` `data_plane` — Docker image passed as `DATA_PLANE_IMAGE` / Dockerfile `ARG` for `nitrum build` and, by default, `nitrum local` (base containing the in-enclave data-plane).
- `[runtime]` `control_plane` — full image ref for the host control-plane on `nitrum cloud deploy` (CloudFormation).
- `[runtime]` `nitro_cli` — image for `nitro-cli` (EIF build and `nitrum describe`).
- `[health_check]` — path, port, and interval for **application** health checks. The data-plane probes `http://127.0.0.1:{port}{path}` and gates `GET /.well-known/enclave/status` on the result (NLB uses that route). While not ready it retries every 500ms; once ready it uses `interval`. Probe timeout and consecutive-failure threshold are platform constants (2s / 3 failures).
- `[scaling]` — replica counts, enclave CPU/RAM, and `instance_type` (Nitro Enclave–capable EC2 type allowlist; default `m6i.xlarge`). `num_cpus` / `ram_size_mib` apply to cloud enclaves and to the local Compose `enclave` service. For zero-downtime rolling, keep `max_replicas >= desired_replicas + 1`.
- `[cloud]` — CloudFormation-only settings (ignored by `nitrum local`):
  - `xray_tracing` — ADOT → X-Ray (default `false`; logs and EMF metrics still export).
  - `log_retention_days` — CloudWatch Logs retention (default `7`).
  - `sns_alarm_topic_arn` — optional SNS topic for unhealthy NLB / low ASG capacity alarms.
  - `safe_rolling` — ASG `MinInstancesInService ≥ 1` and `PauseTime=PT5M` when true (default).
  - `kms_administrator_role_arn` — durable KMS key admin principal (empty → account root). Always re-passed on deploy.
- `[tls_termination]` — `acme` and `domain` for certificates.
- `[egress]` — outbound whitelist enforced inside the data-plane when `enabled = true`:
  - `destinations` — list of regex patterns matched against destination hostnames at DNS query time. Blocked names receive NXDOMAIN; TCP connections to uncached IPs are dropped unless they match implicit platform allows.
  - **Implicit allows** (always merged when egress is enabled): IMDS (`169.254.169.254`), hostnames from `NITRUM_IMDS_BASE_URL` and `NITRUM_*_ENDPOINT_URL`, ACME directory host when `tls_termination.acme` or `NITRUM_ACME_DIRECTORY_URL` is set, regional AWS API endpoints (`kms`, `ssm`, `dynamodb`) using the AWS region resolved during data-plane bootstrap, and the effective OTLP collector endpoint (`NITRUM_OTLP_ENDPOINT` or the platform default).
  - **Environment:** `NITRUM_EGRESS_UPSTREAM_DNS` overrides the upstream resolver (`host:port`) used by the in-enclave DNS proxy (default: first `nameserver` from `/etc/resolv.conf`, typically gvproxy `192.168.127.1:53` on Nitro or Docker `127.0.0.11:53` locally).
  - **Local dev:** the Compose `enclave` service needs `CAP_NET_ADMIN`; rebuild the pebble data-plane with `docker buildx bake data-plane-local` after changes (see [CONTRIBUTING.md](../CONTRIBUTING.md#building-platform-images-for-development)).
  - **Limits:** UDP egress other than DNS is not filtered; IPv6 TCP is not redirected by the transparent proxy and may bypass the whitelist; connections to raw IPs that never went through an allowed DNS lookup are blocked unless they match implicit platform IPs.

Edit `nitrum.toml` to match your app’s port, domain, and infrastructure expectations, then rebuild the EIF and redeploy when you change enclave-related settings.

## Runtime image provenance

Nitrum runtime images published to GHCR (from tagged releases) carry OCI labels for traceability:

| Label | Meaning |
|-------|---------|
| `org.opencontainers.image.revision` | Git commit SHA built into the image |
| `org.opencontainers.image.version` | Release tag (for example `v0.1.0`) |
| `io.nitrum.git.sha` | Same commit SHA (Nitrum-specific) |

Inspect labels on a pulled image:

```bash
docker inspect --format '{{ index .Config.Labels "io.nitrum.git.sha" }}' ghcr.io/OWNER/nitrum/data-plane:v0.1.0
```

**Prefer digest pinning** in `nitrum.toml` for `runtime.data_plane`, `runtime.control_plane`, and `runtime.nitro_cli`:

```toml
# Immutable reference (recommended for production)
data_plane = "ghcr.io/OWNER/nitrum/data-plane@sha256:abcdef..."
```

- **`nitrum init`** resolves `:latest` runtime images to `@sha256:…` automatically when scaffolding a project.
- To pin manually, pull by tag, read the digest, and update `nitrum.toml`:

```bash
docker pull ghcr.io/OWNER/nitrum/data-plane:v0.1.0
docker inspect --format '{{ index .RepoDigests 0 }}' ghcr.io/OWNER/nitrum/data-plane:v0.1.0
```

Floating tags (`:latest`, `:latest-local`) are convenient for local iteration; production stacks should use digests so deploys cannot shift underneath you. See [releases.md](releases.md) for versioning policy.

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
- [releases.md](releases.md) — SemVer, CHANGELOG, CI/release gates, and `nitrum.toml` compatibility.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — developing and testing the Rust workspace.

