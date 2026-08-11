# Contributing

Thanks for your interest in Nitrum. This document describes how to work on the repository and what we expect from contributions.

## Development setup

- **Rust**: MSRV **1.95** ([rustup](https://rustup.rs/)); pinned in [`rust-toolchain.toml`](rust-toolchain.toml) and [`Cargo.toml`](Cargo.toml) `workspace.package.rust-version`.
- **Node.js**: **22** recommended for `packages/node` / `nitrum-node` (engines: `>=18`).
- **Docker**: Used for local enclave builds (`nitrum build`), `nitrum describe`, and `nitrum local` (Compose).
- **Python**: `3.11+` for documentation tooling.
- **Graphviz**: Required to render diagram PNG files (`dot` binary must be on `PATH`).
- **Poetry**: Python dependency manager used for docs diagram generation.
- **make** (optional): Targets in the [`Makefile`](Makefile) — `format`, `lint`, `check` (format+lint), `deny`, `test`, `test-node` / `check-node`, `docs-diagrams`.

Clone the repo and run from the workspace root:

```bash
make check
make test
make deny
```

For the Node napi bindings (run `npm ci` once after clone):

```bash
make check-node
```

### Render architecture diagrams

The architecture diagrams in `docs/diagrams/` are generated from Python source via `mingrammer/diagrams`.

System requirements:

- Python `3.11+`
- Poetry `2.x` (or newer)
- Graphviz (`dot`)

Example install on macOS:

```bash
brew install graphviz poetry
```

Generate diagrams:

```bash
make docs-diagrams
```

This command installs the Poetry diagram dependencies (if needed) and renders:

- `docs/diagrams/output/nitrum-aws-overview.png`
- `docs/diagrams/output/nitrum-runtime-flow.png`

## Code style and quality

Before opening a pull request:

```bash
make check
make deny
make check-node
```

CI runs Rust fmt/clippy/check/test (with `--all-features`), multi-platform `nitrum-node` builds/smoke tests, and `cargo deny` on Linux. Some crates use Linux-only dependencies (for example around Nitro Enclaves networking); if something fails only on your machine, compare with CI logs.

## Releases

See [docs/releases.md](docs/releases.md) for SemVer, `nitrum.toml` breaking-change rules, CHANGELOG requirements, and how tag releases are gated on CI.

## Macro load testing (EC2 Nitro)

Reproducible k6 load against a **deployed** enclave. Prefer a load client in the **same VPC** as the stack. The harness does not deploy or destroy stacks — use `nitrum cloud deploy` / `nitrum cloud destroy` (or `tests/e2e/cloud.sh`) for that.

**Prerequisites on the load client:** [`k6`](https://k6.io/), [`jq`](https://jqlang.github.io/jq/), and a live `ENCLAVE_URL` (NLB HTTPS origin).

```bash
# macOS
brew install k6 jq

# Amazon Linux 2023
sudo dnf install -y https://dl.k6.io/rpm/repo.rpm
sudo dnf install -y k6 jq
```

If you're running these steps within an EC2 instance (recommended), make sure you have `git` installed and that you've cloned the repository first:

```bash
sudo dnf install -y git
git clone https://github.com/matzapata/nitrum
```

**Single run** (default: `GET /.well-known/enclave/status`, `GET /health`, then `POST /crypto`, 50 VUs × 30s):

```bash
ENCLAVE_URL=https://xxxx.elb.us-east-1.amazonaws.com ./tests/perf/run-macro.sh
```

**VU sweep** (capacity curve; run `run-macro.sh` at several concurrencies):

```bash
export ENCLAVE_URL=https://xxxx.elb.us-east-1.amazonaws.com
for vus in 10 50 100 200; do
  PERF_VUS=$vus PERF_ROUTES=crypto \
    PERF_RESULTS_DIR="tests/perf/results/vus-$vus-crypto" \
    ./tests/perf/run-macro.sh
done

for vus in 10 50 100 200; do
  PERF_VUS=$vus PERF_ROUTES=health \
    PERF_RESULTS_DIR="tests/perf/results/vus-$vus-health" \
    ./tests/perf/run-macro.sh
done

for vus in 10 50 100 200; do
  PERF_VUS=$vus PERF_ROUTES=status \
    PERF_RESULTS_DIR="tests/perf/results/vus-$vus-status" \
    ./tests/perf/run-macro.sh
done
```

Useful knobs: `PERF_VUS`, `PERF_DURATION`, `PERF_ROUTES` (`health`, `status`, `crypto`), optional `tests/perf/.env` from `.env.example`. Results land under `tests/perf/results/` (gitignored). After a baseline run, update the **Performance** section in [`README.md`](README.md).

### Local capacity sweep (`nitrum local`)

`nitrum local up` applies `[scaling].num_cpus` and `ram_size_mib` as Compose `cpus` / `mem_limit` on the `enclave` service (same knobs cloud passes to `nitro-cli`). Reuse the k6 harness against `https://127.0.0.1:443`.

Local numbers will **not** match a Nitro deployment (no real enclave, different networking/CPU isolation). Use this only as a quick reference for shape and regressions before you pay for a cloud sweep.

```bash
# Build the local data-plane image
export TAG=dev
export GIT_SHA=$(git rev-parse HEAD)
export IMAGE_PREFIX=docker.io/matzapata   # or your local prefix
export NITRUM_RUNTIME_DATA_PLANE_IMAGE="${IMAGE_PREFIX}/data-plane:${TAG}"
docker buildx bake data-plane-local

# For example, to start the `examples/hello` project:
cargo run -p cli --bin nitrum -- local up --path examples/hello

# Same VU sweep as Nitro, pointed at the local stack
export ENCLAVE_URL=https://127.0.0.1:443
for route in status health crypto; do
  for vus in 10 50 100 200; do
    PERF_VUS=$vus PERF_ROUTES=$route \
      PERF_RESULTS_DIR="tests/perf/results/local-vus-$vus-$route" \
      ./tests/perf/run-macro.sh
  done
done > tests/perf/results/local.txt

# Tear down
cargo run -p cli --bin nitrum -- local down --path examples/hello
```

Compare the local plateau shape (RPS vs VUs) against the Nitro sweep — relative trends matter more than absolute RPS.

### Criterion micro benches

In-process latency benches (no Docker) live under `crates/data-plane/benches/`:

```bash
cargo bench -p data-plane --features bench --bench crypto
cargo bench -p data-plane --features bench --bench ingress
```

These measure single-operation latency and how it scales with payload size (`crypto`: AES-GCM encrypt/decrypt across sizes; `ingress`: full proxy hop including body buffering/drain for `get_empty`, `post_1kiB`, `post_64kiB`). They intentionally don't model concurrency — for throughput/capacity under concurrent load, use the k6 harness above (`tests/perf/run-macro.sh`), which drives real concurrent connections against a real server instead of in-process `oneshot` calls on a single thread.

## Building platform images for development

Runtime images come from `[runtime]` in `nitrum.toml`. Override them for a session with:

- `NITRUM_RUNTIME_DATA_PLANE_IMAGE`
- `NITRUM_RUNTIME_CONTROL_PLANE_IMAGE`
- `NITRUM_RUNTIME_NITRO_CLI_IMAGE`

`nitrum local up` automatically appends `-local` to the resolved `data_plane` tag (for example `${NITRUM_RUNTIME_DATA_PLANE_IMAGE}-local` or `${NITRUM_RUNTIME_DATA_PLANE_IMAGE}:latest-local`). Cloud / `nitrum build` use the resolved image as-is.

Build with the repo-root [`docker-bake.hcl`](docker-bake.hcl):

```bash
export TAG=dev
export GIT_SHA=$(git rev-parse HEAD)
export IMAGE_PREFIX=docker.io/matzapata

# Aws overrides
export AWS_PROFILE=nitrum
export AWS_REGION=us-east-1

# Overrides for cloud.sh running the cli
export NITRUM_RUNTIME_CONTROL_PLANE_IMAGE="${IMAGE_PREFIX}/control-plane:${TAG}"
export NITRUM_RUNTIME_DATA_PLANE_IMAGE="${IMAGE_PREFIX}/data-plane:${TAG}"
export NITRUM_RUNTIME_NITRO_CLI_IMAGE="${IMAGE_PREFIX}/nitro-cli:${TAG}"

# Cloud / EIF path: enclave data-plane + control-plane + nitro-cli (push to a registry).
docker buildx bake --push control-plane data-plane nitro-cli

./tests/e2e/cloud.sh
```

```bash
export TAG=dev
export GIT_SHA=$(git rev-parse HEAD)
export IMAGE_PREFIX=docker.io/matzapata

# Locally we only use data-plane
export NITRUM_RUNTIME_DATA_PLANE_IMAGE="${IMAGE_PREFIX}/data-plane:${TAG}"

# Local Data Plane: uses pebble backend and disables enclave-only features
docker buildx bake data-plane-local

./tests/e2e/local.sh
```

E2E scripts (`tests/e2e/local.sh`, `tests/e2e/cloud.sh`) never build platform images themselves — bake first, then export the overrides.

## Pull requests

- Keep changes focused and describe the motivation in the PR description.
- Reference related issues when applicable.
- For user-visible behavior changes, consider updating [docs/usage.md](docs/usage.md) or [docs/architecture.md](docs/architecture.md).

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project ([MIT](LICENSE)).
