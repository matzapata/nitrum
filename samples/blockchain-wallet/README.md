## Blockchain wallet sample

This sample shows a minimal Ethereum-style wallet running **entirely inside a Nitro enclave**, inspired by the reference in `.cursor/references/nitro-enclave-blockchain-wallet-on-eks/README.md`, but simplified:

- **No DynamoDB / KMS** — the enclave is **stateless**: it never persists keys. Encrypted key blobs are returned to the client, which stores them locally, and encryption/decryption is delegated to Nitrum’s data‑plane crypto endpoints.
- **One-way HMAC** — a shared secret is used on both the client and in-enclave app to protect requests; the secret is **never sent over the wire**, only HMACs of the payload are.
- **Simple Node CLI** — a TypeScript CLI calls the enclave endpoints over HTTPS (via Nitrum), similar to the `samples/hello` client.

### Endpoints

All endpoints are served by the enclave application (see `enclave/src/main.js`):

- `POST /wallet/create`
  - **Request body**:
    - `payload: { nonce: string }`
    - `hmac: string` — `HMAC-SHA256(secret, JSON.stringify(payload))` in hex.
  - **Response**:
    - `{ keyId: string, address: string, encryptedPrivateKey: { iv, tag, ciphertext } }`
  - **Behavior**:
    1. Verifies `hmac` using the shared secret (`HMAC_SECRET` env in the enclave).
    2. Generates a new Ethereum keypair (`ethers.Wallet.createRandom()`).
    3. Sends the private key to the Nitrum data‑plane `encrypt` endpoint (`http://localhost:3000/encrypt`) and receives an encrypted blob.
    4. Returns `keyId`, public `address`, and the encrypted private key blob. The enclave does **not** keep any copy.

- `POST /sign`
  - **Request body**:
    - `payload: { keyId: string, message: string }`
    - `hmac: string` — `HMAC-SHA256(secret, JSON.stringify(payload))` in hex.
  - **Response**:
    - `{ signature: string, address: string }`
  - **Behavior**:
    1. Verifies `hmac` with the shared secret.
    2. Sends the `encryptedPrivateKey` blob to the Nitrum data‑plane `decrypt` endpoint (`http://localhost:3000/decrypt`) and recovers the plaintext private key.
    3. Signs `message` with `ethers.Wallet.signMessage`.
    4. Returns the signature and address. The decrypted key is kept only in memory for the duration of the request.

### HMAC and secrets

- The enclave reads the HMAC secret from `HMAC_SECRET` (environment variable).
- The CLI reads the same value from `BLOCKCHAIN_WALLET_SECRET`.
- For every request:
  - The CLI builds a small JSON `payload`.
  - Computes `hmac = HMAC-SHA256(secret, JSON.stringify(payload))`.
  - Sends `{ payload, hmac }` to the enclave.
  - The enclave recomputes the HMAC and compares using a timing-safe comparison.
- The **secret itself is never included in any request or response**, mirroring the “secret → not being passed, just used in hmac” pattern from the reference design.

### CLI (client)

The client lives under `client/` and is implemented in TypeScript, similar to `samples/hello/client`:

- Depends on `nitrum-node` but does not do attestation in this minimal sample.
- Reads the enclave URL from `ENCLAVE_URL`.
- Uses `BLOCKCHAIN_WALLET_SECRET` to compute HMACs for all requests.

Commands:

- `wallet create`
  - Sends `POST /wallet/create` with a random nonce.
  - Stores `{ keyId, address, encryptedPrivateKey }` in a local `.wallets.json` file next to the CLI.
  - Prints `{ keyId, address }`.
- `sign --key <keyId> --message "<msg>"`
  - Loads the encrypted key blob for `keyId` from `.wallets.json`.
  - Sends `POST /sign` with `{ keyId, message, encryptedPrivateKey }`.
  - Prints `{ signature, address }`.

### Running locally (outline)

From the repository root:

```bash
cd samples/blockchain-wallet

# 1. Build the enclave EIF for this sample
nitrum build

# 2. Set secrets for the enclave (example values)
nitrum cloud env set HMAC_SECRET super-secret-demo-value
nitrum cloud env set KEYSTORE_PASSPHRASE another-demo-passphrase

# 3. Deploy the enclave (similar to samples/hello)
nitrum cloud deploy --eif .nitrum/artifacts/blockchain-wallet.eif
```

Once deployed, Nitrum will output the enclave HTTPS URL. Use it for the CLI:

```bash
cd samples/blockchain-wallet/client

export ENCLAVE_URL="https://<your-enclave-endpoint>"
export BLOCKCHAIN_WALLET_SECRET="super-secret-demo-value"

npm install
npm run build

# Create a wallet
node dist/index.js wallet create

# Sign a message
node dist/index.js sign --key <keyId> --message "hello from nitrum"
```

> **Note**: This sample is intentionally simplified for demonstration:
> - No external persistence (no DynamoDB/KMS); the enclave is stateless and only ever sees encrypted key blobs per request.
> - Minimal request payloads and message signing.
> - HMAC shared secret is configured via env and reused by both CLI and enclave.