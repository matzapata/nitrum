## Nitrum “hello” sample

This is a minimal **end‑to‑end sample** that shows how to:

- Use the **`nitrum` CLI** to build an enclave EIF.
- Inject a simple environment variable into the enclave.
- Deploy the enclave to AWS using **Nitrum’s control-plane** and CloudFormation.

It is intentionally small so you can copy pieces into your own applications.

### Prerequisites

- A working **Nitrum** CLI built from the repo root:
  ```bash
  cargo install --path crates/cli
  ```
- **Docker** installed and running (used by `nitrum build`).
- An AWS account with Nitro Enclaves support and credentials configured in your shell (for `nitrum cloud …` commands).

### Local development (optional)

You can inspect and run the sample locally using the `nitrum` CLI from the repository root:

```bash
cd samples/hello
# For a full local Docker stack when you are iterating on your own app:
# nitrum local up
```

The exact local workflow depends on how you evolve this sample; see the top‑level `docs/usage.md` for the full CLI reference.

### Deploy to AWS

From the **repository root**, build the EIF and deploy the sample enclave:

```bash
cd samples/hello

# 1. Build the enclave image (EIF) for this sample
nitrum build

# 2. Set an example environment variable that will be available inside the enclave
nitrum cloud env set DEMO hello

# 3. Deploy the enclave using the generated EIF artifact
nitrum cloud deploy --eif .nitrum/artifacts/nitrum-hello.eif
```

Once the CloudFormation stack finishes, Nitrum will print the relevant outputs (for example, the load balancer / endpoint URL) that you can use to call the sample service over HTTPS.

### Next steps

- Inspect `nitrum.toml` in this directory to see how the sample is wired to Nitrum’s control-plane and data-plane.
- Replace the sample application code with your own business logic while keeping the `nitrum` configuration and deployment flow.