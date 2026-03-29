# Usage

This guide summarizes the **nitrum** CLI, the configuration file, and what you need installed for each workflow.

## Prerequisites

| Task | Requirements |
|------|----------------|
| Build CLI from source | [Rust](https://rustup.rs/) (stable) |
| `nitrum build`, `nitrum describe` | [Docker](https://docs.docker.com/get-docker/) |
| `nitrum dev` | Docker with Compose support |
| `nitrum deploy` / `nitrum destroy` | [AWS credentials](https://docs.aws.amazon.com/cli/latest/userguide/cli-chap-configure.html) and permissions for CloudFormation, S3, and related resources |
| `nitrum env` | AWS credentials with **SSM** `PutParameter` / `GetParameter` / `DeleteParameter` on `/nitrum/{name}/env/*` (`name` from `nitrum.toml`, same as CloudFormation **`ProjectName`**) |

Install the CLI from a release binary or from the repository root:

```bash
cargo install --path crates/cli
```

The installed binary is **`nitrum`**. The Cargo package name remains **`cli`** for the workspace; build it with `cargo build -p cli` or run without installing via `cargo run -p cli -- <args>` from the repo root.

## Project layout

After `nitrum init <name>`, a typical project contains:

- **`nitrum.toml`** — service name, ports, scaling, TLS, egress, and other settings.
- **`src/`** — your application (for example Node.js).
- **`Dockerfile`** — image used inside the enclave / dev stack.
- **`.nitrum/`** (created when needed) — generated Compose, CloudFormation template copies, and similar artifacts.

## Commands

### `nitrum init [NAME]`

Scaffold a new project with a sample app and default `nitrum.toml`.

### `nitrum build`

Builds the enclave Docker image and produces **`enclave.eif`** in the project directory using **nitro-cli** inside Docker. Requires a valid `nitrum.toml` and project layout.

### `nitrum dev`

Local development via Docker Compose:

- **`nitrum dev up`** — start the stack in the background.
- **`nitrum dev down`** — stop and remove containers.
- **`nitrum dev logs`** — follow service logs.

### `nitrum deploy`

Uploads the EIF (built from source if omitted) and creates or updates the **CloudFormation** stack using the bundled template and your `nitrum.toml`. Useful flags (see `nitrum deploy --help`):

- **`--eif`** — path to an existing EIF file.
- **`--retain`** — retain selected resources on stack delete.

The **`control_plane`** field in `nitrum.toml` is the full Docker image reference (for example `my-registry/nitrum-control-plane:v1`) passed to CloudFormation for the EC2 control-plane service.

### `nitrum destroy`

Tears down deployed AWS resources (see command help for options such as retaining KMS/SSM data).

### `nitrum env`

Manage **application** environment variables as **SSM Parameter Store** `SecureString` values under `/nitrum/{name}/env/{KEY}`, where **`name`** is the **`name`** field in `nitrum.toml` (same value as the CloudFormation stack name and **`ProjectName`** parameter).

- **`nitrum env set KEY VALUE`** — create or overwrite a parameter.
- **`nitrum env get`** — list every app env parameter as `KEY=value` (decrypted; sensitive).
- **`nitrum env delete KEY`** — remove the parameter.

The **data-plane** loads every parameter under that path at startup (unless `NITRUM_APP_ENV_SSM_PREFIX` is set to empty to skip) and passes them to the **user process**, overlaying the parent environment.

### `nitrum describe`

Runs **`nitro-cli describe-eif`** in Docker against an EIF path (wrapper for inspecting measurements and metadata).

## `nitrum.toml` overview

Options are defined in the `shared` crate; the sample project comments point to the source. Common sections:

- **`name`** — project identifier; CloudFormation stack name and **`ProjectName`** match **`name`**; S3 bucket is **`nitrum-{name}`**; SSM paths use **`/nitrum/{name}/…`** (data-plane infra and app env).
- **`data_plane`** — Docker image passed as `DATA_PLANE_IMAGE` / Dockerfile `ARG` for **`nitrum build`** and **`nitrum dev`** (base containing the in-enclave data-plane).
- **`control_plane`** — full image ref for the host control-plane on **`nitrum deploy`** (CloudFormation).
- **`[service]`** — listen port for your app.
- **`[health_check]`** — path, port, and interval for health checks.
- **`[scaling]`** — replica hints and enclave CPU/RAM (used in deployment templates).
- **`[tls_termination]`** — `acme` and `domain` for certificates.
- **`[egress]`** — `enabled` and `destinations` for outbound restrictions (see code and templates for current behavior).

Edit `nitrum.toml` to match your app’s port, domain, and infrastructure expectations, then rebuild the EIF and redeploy when you change enclave-related settings.

## Documentation map

- [architecture.md](architecture.md) — how control-plane, data-plane, and AWS pieces fit together.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — developing and testing the Rust workspace.
