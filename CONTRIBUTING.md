# Contributing

Thanks for your interest in Nitrum. This document describes how to work on the repository and what we expect from contributions.

## Development setup

- **Rust**: A recent stable toolchain ([rustup](https://rustup.rs/)).
- **Docker**: Used for local enclave builds (`nitrum build`), `nitrum describe`, and `nitrum dev` (Compose).
- **just** (optional): Recipes in the [`justfile`](justfile) mirror common commands (`check`, `lint`, `format`, Docker image builds).

Clone the repo and run from the workspace root:

```bash
cargo check --all-targets
cargo test --all-features
```

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
