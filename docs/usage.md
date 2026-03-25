# Usage

This guide summarizes the **nitrum** CLI, the configuration file, and what you need installed for each workflow.

## Prerequisites

| Task | Requirements |
|------|----------------|
| Build CLI from source | [Rust](https://rustup.rs/) (stable) |
| `nitrum build`, `nitrum describe` | [Docker](https://docs.docker.com/get-docker/) |
| `nitrum dev` | Docker with Compose support |
| `nitrum deploy` / `nitrum destroy` | [AWS credentials](https://docs.aws.amazon.com/cli/latest/userguide/cli-chap-configure.html) and permissions for CloudFormation, S3, and related resources |

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

Uploads the EIF (default: `./enclave.eif`) and creates or updates the **CloudFormation** stack using the bundled template and your `nitrum.toml`. Useful flags (see `nitrum deploy --help`):

- **`--eif`** — path to the EIF file.
- **`--region`** — AWS region (overrides environment).
- **`--control-plane-image-tag`** — Docker tag for the control-plane image on instances.
- **`--retain`** — retain selected resources on stack delete.

### `nitrum destroy`

Tears down deployed AWS resources (see command help for options such as retaining KMS/SSM data).

### `nitrum describe`

Runs **`nitro-cli describe-eif`** in Docker against an EIF path (wrapper for inspecting measurements and metadata).

## `nitrum.toml` overview

Options are defined in the `shared` crate; the sample project comments point to the source. Common sections:

- **`name`** — project identifier; used for derived resource names (for example S3 bucket naming).
- **`[service]`** — listen port for your app.
- **`[health_check]`** — path, port, and interval for health checks.
- **`[scaling]`** — replica hints and enclave CPU/RAM (used in deployment templates).
- **`[tls_termination]`** — `acme` and `domain` for certificates.
- **`[egress]`** — `enabled` and `destinations` for outbound restrictions (see code and templates for current behavior).

Edit `nitrum.toml` to match your app’s port, domain, and infrastructure expectations, then rebuild the EIF and redeploy when you change enclave-related settings.

## Documentation map

- [architecture.md](architecture.md) — how control-plane, data-plane, and AWS pieces fit together.
- [CONTRIBUTING.md](../CONTRIBUTING.md) — developing and testing the Rust workspace.
