# CLI project template

Customer scaffold copied by `nitrum init`. Uses a git dependency on `nitrum-sdk` and a project-directory Docker build context.

Also holds the bundled stack YAML:

- `local-stack.yml` — Compose template for `nitrum local` (rewritten on every `up`)
- `cloud-stack.yml` — CloudFormation template for `nitrum cloud deploy`. Copy into a project with `nitrum cloud eject` (not rewritten on later deploys).

For a monorepo demo that path-depends on `crates/sdk`, see [`examples/hello`](../../../examples/hello).

CLI and local development: [docs/usage.md](../../../docs/usage.md).
