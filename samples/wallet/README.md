## Blockchain wallet sample

This sample is a small **Ethereum-style signing helper** that runs inside a Nitro enclave, in the spirit of [aws-samples/nitro-enclave-blockchain-wallet-on-eks](https://github.com/aws-samples/nitro-enclave-blockchain-wallet-on-eks), but wired through Nitrum’s build and deploy flow.

The enclave uses the Nitrum **data-plane** for `encrypt`, `decrypt`, `random`, and `kv/set` + `kv/get` (see `docs/usage.md`). After each successful `/wallet` creation the sample persists the same ciphertext returned to the client under the logical key `wallet:demo_last_ciphertext` so the DynamoDB-backed KV path is exercised; if that KV write fails, the whole request fails and no ciphertext is returned. Application code lives in `enclave/src/main.js` with OpenTelemetry metrics in `enclave/src/instrumentation.js` (`app.wallet.created`, `app.wallet.sign.duration.ms`).

### Endpoints

All HTTP routes are served by the enclave app:

- `GET /health`  
Returns plain text `OK` (matches `nitrum.toml` `[health_check]`).
- `POST /wallet`  
  - **Body**: `{ "secret": "<string>" }` — client-chosen string (treat like a password or HMAC secret) stored alongside the key material inside the encrypted blob.  
  - **Response**: `{ "ciphertext": "<base64 string>" }` — same format as the data-plane `encrypt` response `data` field, wrapping `{ privateKey, secret }` (decryptable only inside the enclave via the data-plane).  
  - **Behavior**:
    1. Calls `POST http://localhost:3000/random` with `{ length: 32 }` and uses the bytes as the secp256k1 private key material.
    2. Encrypts `JSON.stringify({ privateKey: "<hex>", secret })` via `POST http://localhost:3000/encrypt`.
    3. Writes the ciphertext to KV under `wallet:demo_last_ciphertext` via `POST http://localhost:3000/kv/set`.
    4. Returns the ciphertext in the JSON body (the client still passes this blob to `/wallet/sign`).
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

