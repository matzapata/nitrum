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

`nitrum init [name]` by default creates a sample Node.js server, but you can use any language or stack as long as the project includes a **Dockerfile** that exposes the **application port** from `[project].port` in `nitrum.toml` and runs the data-plane with your bundled `nitrum.toml` (for example `CMD ["/app/data-plane", "--config", "/app/nitrum.toml"]`). The data-plane reads `[project].start_command` from that file and starts the user process.

The repository includes a reference project under `samples/hello` which shows the end‑to‑end flow:

- Build the EIF: `nitrum build`.
- Set a simple environment variable: `nitrum cloud env set DEMO hello`.
- Deploy the enclave: `nitrum cloud deploy`

Use that sample as a concrete reference when wiring your own projects.

### Ingress HTTPS API (external, `/.well-known/...`)

The in-enclave **data-plane** terminates **TLS** on `NITRUM_INGRESS_LISTEN_ADDR` (default `**0.0.0.0:443`**). Clients reach these URLs over **HTTPS** (for example `https://nitrum.local` in the local Compose stack, or your deployed domain). Requests on the paths below are handled **inside the ingress** when enabled via `[well_known]`; everything else is **reverse-proxied** over HTTP to your app at the port from `[project].port` in `nitrum.toml`.

#### Endpoints reachable from the Internet (or local TLS client)

When `[well_known].enclave_status` is true (default):

- `GET /.well-known/enclave/status` — minimal liveness payload for the ingress/data-plane. On success: `200 OK` and `{"status":"ok"}`.

When `[well_known].enclave_attestation` is true (default):

- `GET /.well-known/enclave/attestation` — **AWS Nitro attestation document** for the running enclave, with the **current TLS leaf certificate** bound into the NSM request (certificate hash as `public_key` material). Optional query: `nonce` (standard Base64 of raw nonce bytes). On success: `200 OK` and `{"data":"<base64-encoded attestation document>"}`. If the TLS certificate is not yet available: `503` with `{"error":"TLS certificate not yet available"}`. Attestation errors may return `500` with `{"error":"attestation failed: ..."}`.

When `[tls_termination].acme` is enabled:

- `GET /.well-known/acme-challenge/{token}` — **ACME HTTP-01** on a separate plain-HTTP listener (`NITRUM_ACME_HTTP01_LISTEN_ADDR`, default `0.0.0.0:80`). On success: `200 OK` and the challenge key authorization bytes (`application/octet-stream`).

### Data-plane crypto HTTP API (internal)

When you run `nitrum local up` or deploy with `nitrum cloud deploy`, the in-enclave data-plane serves a small JSON HTTP API for encryption, key-value storage, and randomness on `NITRUM_CRYPTO_API_LISTEN_ADDR` (default `**0.0.0.0:3000`**). Application code inside the enclave typically calls `http://localhost:3000/...`.

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
- `POST /kv/set` Stores a UTF-8 string under a logical key in shared DynamoDB storage. The value is encrypted with the same KMS-backed DEK as `/encrypt` before persistence (`set_object` overwrites any existing value for that key).  
  Request body:
  ```
  {
    "key": "<logical key>",
    "value": "<UTF-8 string>"
  }
  ```
  The logical `key` must be non-empty, at most 512 bytes, and may contain only ASCII letters, digits, and `_ : @ . / -`. The `value` must not exceed 384 KiB (UTF-8 byte length).  
  Success: `200 OK` with `{ "data": "ok", "error": null }`.  
  Validation errors: `400 Bad Request`. Encryption or storage failures: `500 Internal Server Error`.
- `POST /kv/get` Loads and decrypts a value previously stored with `/kv/set`.  
  Request body:
  ```
  {
    "key": "<logical key, same rules as /kv/set>"
  }
  ```
  Success: `200 OK` with `{ "data": "<original plaintext string>", "error": null }`.  
  Missing key: `404 Not Found`. Invalid key: `400 Bad Request`. Decryption failure or non-UTF-8 plaintext after decrypt: `422 Unprocessable Entity`. Storage errors: `500 Internal Server Error`.
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

The `samples/hello/src/main.js` file demonstrates an encrypt/decrypt round-trip, `/random`, and `POST /kv` (which calls `/kv/set` and `/kv/get` on the data-plane). The `samples/wallet/enclave/src/main.js` sample uses the same crypto and KV endpoints and persists each new wallet ciphertext under `wallet:demo_last_ciphertext` while still returning it in the HTTP response for the client-driven signing flow.

## Commands

### `nitrum init [NAME]`

Scaffold a new project with a sample app, default `nitrum.toml`, and `tests/integration.test.mjs` (Node’s test runner + optional `nitrum-node` attestation checks when `ENCLAVE_URL` points at a real deployment).

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

- `nitrum local up` — start the stack in the background.
- `nitrum local down` — stop and remove containers.
- `nitrum local logs` — follow service logs.

The enclave Dockerfile’s `DATA_PLANE_IMAGE` build arg comes from the environment variable `NITRUM_LOCAL_DATA_PLANE_IMAGE` if set; otherwise it defaults to `ghcr.io/matzapata/nitrum/data-plane:latest-dev`. Use a local or Pebble-enabled build when you are not using that default (for example the image produced by `tests/e2e/local.sh`).

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
- `nitrum cloud env delete KEY` — remove the parameter (add `--force` to skip the confirmation prompt, for scripts).

At runtime, the data-plane loads every parameter under that path at startup (unless `NITRUM_APP_ENV_SSM_PREFIX` is set to empty to skip) and passes them to the user process environment, overlaying the parent environment.

### `nitrum cloud logs`

Stream or poll CloudWatch Logs for the deployed control-plane or data-plane:

```bash
nitrum cloud logs --service control-plane   # default
nitrum cloud logs --service data-plane
```

Useful flags: `--follow`, `--since-minutes`, `--filter` (CloudWatch filter pattern).

This is the main way to debug enclaves in the field: look for TLS/ACME, KMS, or app‑level errors in these streams.

## Observability and logging

Nitrum control-plane and data-plane share the [`observability`](../crates/observability) crate: structured fields, optional JSON output, CloudWatch export, and redaction of sensitive values before logs are written.

### CloudWatch log groups

| Group | Writer | Contents |
|-------|--------|----------|
| `/nitrum/{project}/control-plane` | Host Docker `control-plane` | gvproxy, enclave supervisor, platform stderr |
| `/nitrum/{project}/data-plane` | In-enclave `data-plane` | Ingress, KMS, ACME, crypto API, and **app** stdout/stderr |

Log streams are named `{service}-{suffix}` (for example `control-plane-12345` on the host, `data-plane-i-0abc-42` inside the enclave using instance id and PID).

### Environment variables

| Variable | Where | Meaning |
|----------|--------|---------|
| `RUST_LOG` | control-plane, data-plane | `tracing` filter (default `info`; e.g. `info,app=debug`) |
| `NITRUM_LOG_FORMAT` | both | `json` (production) or `human` (local); data-plane images default to `json` |
| `NITRUM_PROJECT_NAME` | control-plane host | Enables CloudWatch on the host when set (CloudFormation sets this to `project.name`) |
| `NITRUM_INSTANCE_ID` | control-plane host | EC2 instance id metric dimension (userdata sets this from IMDS; default `local`) |
| `NITRUM_METRICS_FLUSH_SECS` | both | EMF flush interval in seconds (default `60`) |
| `NITRUM_PROMETHEUS` | control-plane host only | Set to `1` to expose Prometheus text on `NITRUM_PROMETHEUS_LISTEN` (default `127.0.0.1:9090`; not used in the enclave) |
| `NITRUM_PROMETHEUS_LISTEN` | control-plane host | Listen address for `/metrics` when `NITRUM_PROMETHEUS=1` |

The data-plane does not require `NITRUM_PROJECT_NAME`; it reads `project.name` from `nitrum.toml` bundled in the EIF and exports to `/nitrum/{project}/data-plane` when AWS credentials are available (IMDS inside the enclave).

### CloudWatch custom metrics (EMF)

Nitrum publishes custom metrics to namespace **`Nitrum/{project.name}`** using [Embedded Metric Format](https://docs.aws.amazon.com/AmazonCloudWatch/latest/monitoring/CloudWatch_Embedded_Metric_Format.html) on dedicated log streams (`{component}-{suffix}-metrics`). No `cloudwatch:PutMetricData` IAM permission is required—only the existing CloudWatch Logs policy.

**Dimensions on every series:** `ProjectName`, `Component` (`control-plane` or `data-plane`), `InstanceId` (IMDS on EC2; `local` in Compose).

| Metric | Component | Use |
|--------|-----------|-----|
| `EnclaveRunning` | control-plane | Gauge `0`/`1` from enclave supervisor poll |
| `EnclaveRestartCount` | control-plane | Counter on each successful enclave (re)start |
| `IngressRequests` | data-plane | Counter per proxied request |
| `Ingress5xx` | data-plane | Counter when ingress returns HTTP 5xx |
| `AcmeCertDaysRemaining` | data-plane | Gauge: days until leaf cert `notAfter` (when ACME enabled) |
| `KmsErrors` | data-plane | Counter on KMS API failures |
| `DynamoDbErrors` | data-plane | Counter on DynamoDB API failures (not lock contention) |
| `LeaderLockHeld` | data-plane | Gauge `0`/`1` with dimension `LockKey` = `crypto` or `acme` |
| `AppHealthCheckPass` | data-plane | Gauge `0`/`1` from `[health_check]` probe to the user app |

**Dashboard:** CloudFormation creates **`{project.name}-ops`**. The canonical widget definition is mirrored in [`crates/cli/dashboards/ops.json`](../crates/cli/dashboards/ops.json) and duplicated in `stack.yml` for CloudFormation deployment.

**Alarm:** **`{project.name}-enclave-down`** fires when `EnclaveRunning` averages below `0.5` for five consecutive one-minute periods (`TreatMissingData: breaching`). Pass stack parameter **`AlarmEmail`** to subscribe an SNS email notification; leave empty to create the alarm without SNS. After deploy, the recipient must confirm the SNS subscription email before notifications are delivered.

**NLB vs application health:** the NLB target group uses TCP checks on port 443. `AppHealthCheckPass` reflects your app’s `[health_check]` HTTP probe—use both to detect “TLS up but app broken” scenarios.

#### Post-deploy verification

1. Open the **`{project.name}-ops`** dashboard in CloudWatch (region matches your stack).
2. Confirm **`EnclaveRunning`** is `1` after the enclave warms up (allow one to two EMF flush intervals, default 60s).
3. Send HTTPS traffic through the NLB and confirm **`IngressRequests`** increases.
4. With ACME enabled, confirm **`AcmeCertDaysRemaining`** is positive.
5. Optional: stop the enclave (`nitro-cli terminate-enclave` on the host) and confirm **`EnclaveRunning`** drops to `0` and **`{project.name}-enclave-down`** enters `ALARM` after about five minutes.

#### Local Prometheus (control-plane only)

When running the control-plane binary with `NITRUM_PROMETHEUS=1`, scrape `http://127.0.0.1:9090/metrics` for the same in-process counters and gauges (development only; production uses CloudWatch EMF).

### Structured fields

Every log line includes:

- `project` — `project.name` from `nitrum.toml`
- `component` — `control-plane`, `data-plane`, or `app`

Ingress requests add `request_id` (from `x-request-id` or a generated UUID). Failures should include `error.kind` (for example `ingress_proxy`, `kms_decrypt`, `acme_state`).

### Platform logs vs application logs

The data-plane forwards user process stdout/stderr with `tracing` target `app`. Those events appear in the **same** log group as platform logs (`/nitrum/{project}/data-plane`) with `component=app`. Platform code uses `component=data-plane`.

**CloudWatch Logs Insights — application only:**

```
fields @timestamp, project, component, request_id, @message
| filter component = "app"
| sort @timestamp desc
| limit 50
```

**Platform / data-plane errors:**

```
fields @timestamp, `error.kind`, @message
| filter component = "data-plane" and ispresent(`error.kind`)
| sort @timestamp desc
| limit 50
```

**Ingress by request id:**

```
fields @timestamp, request_id, @message
| filter ispresent(request_id) and @message like /ingress/
| sort @timestamp desc
```

### Redaction

Logs never include plaintext DEK material, TLS private keys, `Authorization` / `Cookie` header values, or raw KMS ciphertext blobs in debug paths. Field names such as `plaintext`, `ciphertext`, and `private_key` are scrubbed. See unit tests in `crates/observability/src/redact.rs`.

### Local development

Without CloudWatch (no `NITRUM_PROJECT_NAME` on the host, or no AWS reachability in the enclave), logs go to stderr only. Use `nitrum local logs` for Compose output, or set `NITRUM_LOG_FORMAT=human` and `RUST_LOG=debug` for readable local traces.

### `nitrum describe`

Runs `nitro-cli describe-eif` in Docker against an EIF path (wrapper for inspecting measurements and metadata). Use this to pull out PCRs you want to enforce from your verification or KMS policies.

## `nitrum.toml` overview

Options are defined in the `shared` crate; the sample project comments point to the source. Common sections:

- `[project]` `name` — project identifier; CloudFormation stack name and `ProjectName` match it; S3 bucket is `nitrum-{name}`; SSM paths use `/nitrum/{name}/…` (data-plane infra and app env).
- `[project]` `port` — TCP port your app listens on at `127.0.0.1` (ingress proxies here after TLS).
- `[project]` `start_command` — argv for the user process (JSON array in `nitrum.toml`); the data-plane spawns it after loading config (CLI args after `--` still override when used).
- `[runtime]` `data_plane` — Docker image passed as `DATA_PLANE_IMAGE` / Dockerfile `ARG` for `nitrum build` and, by default, `nitrum local` (base containing the in-enclave data-plane).
- `[runtime]` `control_plane` — full image ref for the host control-plane on `nitrum cloud deploy` (CloudFormation).
- `[runtime]` `nitro_cli` — image for `nitro-cli` (EIF build and `nitrum describe`).
- `[well_known]` — `enclave_status` / `enclave_attestation` toggle the `/.well-known/enclave/*` routes on the TLS listener (defaults: enabled).
- `[health_check]` — path, port, and interval for health checks.
- `[scaling]` — replica hints and enclave CPU/RAM (used in deployment templates).
- `[tls_termination]` — `acme` and `domain` for certificates.
- `[egress]` — `enabled` and `destinations` for outbound restrictions (see code and templates for current behavior).

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

Floating tags (`:latest`, `:latest-dev`) are convenient for local iteration; production stacks should use digests so deploys cannot shift underneath you. See [releases.md](releases.md) for versioning policy.

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

