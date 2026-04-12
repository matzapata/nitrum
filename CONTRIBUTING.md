# Contributing

Thanks for your interest in Nitrum. This document describes how to work on the repository and what we expect from contributions.

## Development setup

- **Rust**: A recent stable toolchain ([rustup](https://rustup.rs/)).
- **Docker**: Used for local enclave builds (`nitrum build`), `nitrum describe`, and `nitrum local` (Compose).
- **Python**: `3.11+` for documentation tooling.
- **Graphviz**: Required to render diagram PNG files (`dot` binary must be on `PATH`).
- **Poetry**: Python dependency manager used for docs diagram generation.
- **just** (optional): Recipes in the [`justfile`](justfile) mirror common commands (`check`, `lint`, `format`, Docker image builds, docs diagram generation).

Clone the repo and run from the workspace root:

```bash
cargo check --all-targets
cargo test --all-features
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
just generate-diagrams
```

This command installs the Poetry diagram dependencies (if needed) and renders:

- `docs/diagrams/output/nitrum-aws-overview.png`
- `docs/diagrams/output/nitrum-runtime-flow.png`

## Code style and quality

Before opening a pull request:

```bash
cargo fmt --all
cargo clippy --all-targets --all-features
```

CI runs `cargo fmt --check`, `cargo clippy … -- -D warnings`, `cargo check`, and `cargo test` on Linux. Some crates use Linux-only dependencies (for example around Nitro Enclaves networking); if something fails only on your machine, compare with CI logs.

## Pull requests

- Keep changes focused and describe the motivation in the PR description.
- Reference related issues when applicable.
- For user-visible behavior changes, consider updating [docs/usage.md](docs/usage.md) or [docs/architecture.md](docs/architecture.md).

## License

By contributing, you agree that your contributions will be licensed under the same terms as the project ([MIT](LICENSE)).
