## Nitrum “hello” example

Minimal Rust enclave app that exercises the data-plane crypto API via workspace [`crates/sdk`](../../crates/sdk), egress allowlisting, and OpenTelemetry metrics.

This is an **in-repo** demo (same project-dir Docker layout as [`crates/cli/template`](../../crates/cli/template); workspace `[patch]` maps git `nitrum-sdk` to local `crates/sdk` for `cargo` builds). The scaffold `nitrum init` copies is [`crates/cli/template`](../../crates/cli/template).

For **requirements**, **local development** (`nitrum local …`), and CLI details, see [docs/usage.md](../../docs/usage.md).

```bash
cd examples/hello
nitrum local up   # or nitrum build / cloud deploy
```

`start_command` runs `/app/hello`. Metrics (`app.crypto.ops`, `app.kv.duration.ms`) export when `OTEL_EXPORTER_OTLP_ENDPOINT` is injected by the data-plane runner.

Integration tests (JS + `nitrum-node`) live under `tests/`.
