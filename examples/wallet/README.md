# Blockchain wallet example

Rust enclave app that generates an encrypted wallet key via the Nitrum crypto API
and signs EIP-1559 transactions after HMAC proof verification.

Uses workspace [`crates/sdk`](../../crates/sdk) for `encrypt`, `decrypt`, `random`,
and `kv/set` (git `nitrum-sdk` dep + workspace `[patch]` to the local crate; Docker build
context is the project directory). After each successful `/wallet` creation it persists
`wallet:demo_last_ciphertext`. Metrics: `app.wallet.created`, `app.wallet.sign.duration.ms`.

`nitrum init` does not scaffold this project — see [`crates/cli/template`](../../crates/cli/template) for the customer hello template.

```bash
cd examples/wallet
nitrum local up   # or nitrum build / cloud deploy

# Against a running enclave:
ENCLAVE_URL=https://nitrum.local ENCLAVE_TLS_INSECURE=1 npm test
```

Integration tests live under `tests/` (attestation when not local-dev TLS, `/health`,
create wallet + sign flow).
