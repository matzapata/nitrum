# Usage

This guide summarizes the **nitrum** CLI, the configuration file, and what you need installed for each workflow.

## Prerequisites

| Task | Requirements |
|------|----------------|
| Build CLI from source | [Rust](https://rustup.rs/) (stable) |
| `nitrum build`, `nitrum describe` | [Docker](https://docs.docker.com/get-docker/) |
| `nitrum local` | Docker with Compose support |
| `nitrum cloud deploy` / `nitrum cloud destroy` | [AWS credentials](https://docs.aws.amazon.com/cli/latest/userguide/cli-chap-configure.html) and permissions for CloudFormation, S3, and related resources |
| `nitrum cloud env` | AWS credentials with **SSM** `PutParameter` / `GetParameter` / `DeleteParameter` on `/nitrum/{name}/env/*` (`name` = `project.name` in `nitrum.toml`, same as CloudFormation **`ProjectName`**) |
| `nitrum cloud logs` | AWS credentials with **CloudWatch Logs** read access for `/nitrum/{name}/…` log groups |

Install the CLI from a release binary or from the repository root:

```bash
cargo install --path crates/cli
```

The installed binary is **`nitrum`**. The Cargo package name remains **`cli`** for the workspace; build it with `cargo build -p cli` or run without installing via `cargo run -p cli -- <args>` from the repo root.

## Project layout

After `nitrum init <name>`, a typical project contains:

- **`nitrum.toml`** — `project`, `runtime` images, service port, scaling, TLS, egress, and other settings.
- **`src/`** — your application (for example Node.js).
- **`Dockerfile`** — image used inside the enclave / dev stack.
- **`.nitrum/`** (created when needed) — generated Compose, CloudFormation template copies, and similar artifacts.

## Commands

### `nitrum init [NAME]`

Scaffold a new project with a sample app and default `nitrum.toml`.

### `nitrum build`

Builds the enclave Docker image and produces **`.nitrum/artifacts/{name}.eif`** in the project directory using **nitro-cli** inside Docker. Requires a valid `nitrum.toml` and project layout.

### `nitrum local`

Local development via Docker Compose:

- **`nitrum local up`** — start the stack in the background.
- **`nitrum local down`** — stop and remove containers.
- **`nitrum local logs`** — follow service logs.

### `nitrum cloud deploy`

Uploads the EIF (built from source if omitted) and creates or updates the **CloudFormation** stack using the bundled template and your `nitrum.toml`. Useful flags (see `nitrum cloud deploy --help`):

- **`--eif`** — path to an existing EIF file.
- **`--retain`** — retain selected resources on stack delete.
- **`--kms-administrator-role-arn`** — optional full IAM role or user ARN passed through as CloudFormation **`KmsAdministratorRoleArn`**. If you omit the flag, that parameter is not supplied and the template default applies. To look up your **account ID** when building ARNs, run: `aws sts get-caller-identity | jq -r '.Account'`.

The **`runtime.control_plane`** field in `nitrum.toml` is the full Docker image reference (for example `my-registry/nitrum-control-plane:v1`) passed to CloudFormation for the EC2 control-plane service.

### `nitrum cloud destroy`

Tears down deployed AWS resources (see command help for options such as retaining KMS/SSM data).

### `nitrum cloud env`

Manage **application** environment variables as **SSM Parameter Store** `SecureString` values under `/nitrum/{name}/env/{KEY}`, where **`name`** is **`project.name`** in `nitrum.toml` (same value as the CloudFormation stack name and **`ProjectName`** parameter).

- **`nitrum cloud env set KEY VALUE`** — create or overwrite a parameter.
- **`nitrum cloud env get`** — list every app env parameter as `KEY=value` (decrypted; sensitive).
- **`nitrum cloud env delete KEY`** — remove the parameter.

### `nitrum cloud logs`

Stream or poll **CloudWatch Logs** for the deployed data-plane or control-plane (`--service data-plane` or `control-plane`).

The **data-plane** loads every parameter under that path at startup (unless `NITRUM_APP_ENV_SSM_PREFIX` is set to empty to skip) and passes them to the **user process**, overlaying the parent environment.

### `nitrum describe`

Runs **`nitro-cli describe-eif`** in Docker against an EIF path (wrapper for inspecting measurements and metadata).

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

## Documentation map

- [architecture.md](architecture.md) — how control-plane, data-plane, and AWS pieces fit together.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — developing and testing the Rust workspace.
