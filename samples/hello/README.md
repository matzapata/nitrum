## Nitrum “hello” sample

A minimal project that builds an EIF, injects env vars, and deploys with Nitrum’s control-plane and CloudFormation—small enough to copy into your own apps.

For **requirements**, **local development** (`nitrum local …`), **workflow**, and every command in detail, see [docs/usage.md](../../docs/usage.md).

From the repo root, install the CLI once (`cargo install --path crates/cli`), then:

```bash
cd samples/hello
nitrum build
nitrum cloud env set DEMO hello
nitrum cloud deploy
```

After deploy, use the stack outputs (for example the load balancer URL) to reach the service over HTTPS.

Inspect `nitrum.toml` here for how this sample wires the data-plane and control-plane; replace `enclave/` with your own code when you are ready.