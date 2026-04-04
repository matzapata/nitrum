## Blockchain wallet sample

This sample is a small **Ethereum-style signing helper** that runs inside a Nitro enclave, in the spirit of [aws-samples/nitro-enclave-blockchain-wallet-on-eks](https://github.com/aws-samples/nitro-enclave-blockchain-wallet-on-eks), but wired through Nitrum’s build and deploy flow.

The enclave uses the Nitrum **data-plane** for `encrypt`, `decrypt`, and `random` (see `docs/usage.md`). Application code lives in `enclave/src/main.js`.

### Endpoints

All HTTP routes are served by the enclave app:

- `GET /health`  
Returns plain text `OK` (matches `nitrum.toml` `[health_check]`).
- `POST /wallet`  
  - **Body**: `{ "secret": "<string>" }` — client-chosen string (look at password derivation algorithms for ) stored alongside the key inside the encrypted blob 
  - **Response**: `{ "ciphertext": <object> }` — opaque JSON from the data-plane `encrypt` endpoint wrapping `{ privateKey, secret } (can only be decrypted by enclave)`.  
  - **Behavior**:
    1. Calls `POST http://localhost:3000/random` with `{ length: 32 }` and uses the bytes as the secp256k1 private key material.
    2. Encrypts `JSON.stringify({ privateKey: "<hex>", secret })` via `POST http://localhost:3000/encrypt`.
    3. Returns the ciphertext. The enclave does not retain a copy after the response.
- `POST /wallet/sign`  
  - **Body**:
    - `ciphertext` — same object returned from `/wallet`.
    - `txData` — EIP-1559-style fields as JSON numbers/strings: `to`, `nonce`, `chainId`, `gasLimit`, `maxFeePerGas`, `maxPriorityFeePerGas`, `value` (wei).
    - `proof` — hex-encoded `HMAC-SHA256(secret, JSON.stringify(txData))`, where `secret` is the UTF-8 string from wallet creation and `txData` is serialized.
  - **Response**: `{ "signature": "<hex>" }` — RLP-signed type-2 transaction 
  - **Behavior**:
    1. Decrypts `ciphertext` via `POST http://localhost:3000/decrypt`.
    2. Parses JSON, recomputes the HMAC over `txData`, and compares to `proof` (string equality).
    3. Signs the transaction; decrypted material exists only for the lifetime of the request.

### Client (`client/`)

The script creates a wallet via `POST /wallet`, builds fake `txData`, computes `proof`, then calls `POST /wallet/sign` and prints the signed transaction hex.

### Deploy (outline)

Project name in `nitrum.toml` is `blockchain-wallet`, so the default EIF path is `.nitrum/artifacts/blockchain-wallet.eif`.

From the repo:

```bash
cd samples/wallet
nitrum build
nitrum cloud deploy
```

When the stack is ready, point the client at the HTTPS URL Nitrum prints (and remove or replace TLS-insecure settings for anything real).

