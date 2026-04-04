# nitrum-node

Verify [AWS Nitro Enclave](https://docs.aws.amazon.com/enclaves/latest/user/nitro-enclaves-concepts.html) attestation documents in TypeScript.

This package is part of the **Nitrum** project, which provides a Rust data-plane, control-plane, and CLI for running applications inside Nitro Enclaves. See the repository’s `README.md` and `docs/architecture.md` for a broader overview of how attestation fits into the system.

## Installation

```bash
npm install nitrum-node
```

## Usage

For a Nitrum enclave over HTTPS, use **`NitrumVerifier`**. It calls **`GET /.well-known/enclave/attestation?nonce=...`** with a fresh nonce, verifies the COSE document and certificate chain, enforces freshness (default **5 minutes**), pins PCRs if you ask it to, and confirms the TLS leaf certificate matches the attestation’s `public_key` (**`SHA-256(DER(leaf))`**, 32 bytes).

```ts
import { NitrumVerifier } from 'nitrum-node';

const verifier = new NitrumVerifier({
  baseUrl: 'https://your-enclave.example',
  expectedPcrs: { pcr0: EXPECTED_PCR0 },
});

const document = await verifier.verify();

console.log('Attestation valid ✓', {
  module_id: document.module_id,
  timestamp: document.timestamp,
});
```

### Options on `NitrumVerifier`

| Option | Purpose |
|--------|---------|
| `expectedPcrs` | Each listed key (e.g. `pcr0`) must match the given hex (case-insensitive). |
| `maxAgeMs` | Max age of the document timestamp vs `Date.now()` (default: `300_000`). |
| `trustedRoot` | Override the embedded AWS Nitro root CA (PEM or DER). |
| `debug` | When `true`, attestation failures from the inner verifier include `error`. |
| `tlsRejectUnauthorized` | Node TLS verification when reading the leaf cert (default: `true`; set `false` only for local debugging). |

```ts
const verifier = new NitrumVerifier({
  baseUrl: 'https://your-enclave.example',
  expectedPcrs: { pcr0: EXPECTED_PCR0 },
  maxAgeMs: 5 * 60_000,
  trustedRoot: myCustomRootPem,
  debug: true,
});
```

`verify()` returns `Promise<ParsedAttestation>` or throws on HTTP, TLS, or verification errors.

### Debug failures

On failure, `verify()` throws an `Error` whose message is the human-readable reason. With **`debug: true`**, attestation failures from the inner verifier set **`error.cause`** to the underlying value (same as `AttestationFailure.error` from `verifyAttestation`).

## Low-level: raw bytes (`verifyAttestation`)

If you already have the **raw binary** attestation document (for example from another transport), use **`verifyAttestation`**. It does **not** fetch over HTTP and does **not** check TLS leaf binding — use **`verifyTlsLeafBindsAttestation`** afterward if you need the same `public_key` ↔ certificate tie-in as `NitrumVerifier`.

```ts
import { verifyAttestation, verifyTlsLeafBindsAttestation } from 'nitrum-node';

const result = await verifyAttestation(rawDocument, {
  expectedPcrs: { pcr0: EXPECTED_PCR0 },
  maxAgeMs: 5 * 60_000,
  nonce: myNonceBytes, // optional: require document nonce to match
});

if (!result.valid) {
  console.error('Invalid attestation:', result.reason, result.error);
  return;
}

const binds = await verifyTlsLeafBindsAttestation(
  result.document,
  tlsLeafCertificateDer,
);
if (!binds) throw new Error('TLS leaf does not match attestation public_key');
```

### Custom trusted root (low-level)

By default the official [AWS Nitro root CA](https://aws-nitro-enclaves.amazonaws.com/AWS_NitroEnclaves_Root-G1.zip) is embedded as **`AWS_NITRO_ROOT_CA`**. Override with `trustedRoot` on `NitrumVerifier` or `verifyAttestation`:

```ts
import { verifyAttestation, AWS_NITRO_ROOT_CA } from 'nitrum-node';

const result = await verifyAttestation(document, {
  trustedRoot: myCustomRootPem,
});
```

## What is validated

| # | Check |
|---|-------|
| 1 | Input decodes as COSE_Sign1 (CBOR array or tagged structure) |
| 2 | Payload parses as an attestation document with required fields |
| 3 | COSE signature is valid against the embedded signing certificate |
| 4 | Signing certificate was issued by the CA bundle in the document |
| 5 | CA bundle forms a valid chain up to the trusted root CA |
| 6 | If `options.nonce` is set: document nonce matches (constant-time) |
| 7 | If `options.maxAgeMs` is set: document timestamp is within freshness bound |
| 8 | If `options.expectedPcrs` is non-empty: listed PCRs match expected hex |

**`NitrumVerifier.verify()`** runs the same attestation checks (with its own nonce and default `maxAgeMs`) and **additionally** confirms the TLS leaf DER hashes to `public_key`.

> **Image trust is your policy.**  
> This library confirms the attestation is cryptographically authentic. Pin PCRs via `expectedPcrs` for the enclave image(s) you trust.

## API

### `NitrumVerifier`

Constructor options:

| Field | Type | Description |
|-------|------|-------------|
| `baseUrl` | `string` | Enclave origin; HTTPS required for TLS binding. Trailing `/` optional. |
| `expectedPcrs` | `Record<string, string>` | Optional PCR pin (hex, case-insensitive). |
| `maxAgeMs` | `number` | Freshness bound (default: `300_000`). |
| `tlsRejectUnauthorized` | `boolean` | Node TLS verification when fetching leaf cert (default: `true`, unless `NODE_TLS_REJECT_UNAUTHORIZED=0`). |
| `trustedRoot` | `string \| Uint8Array` | Override trusted root CA. |
| `debug` | `boolean` | Pass through to inner `verifyAttestation`. |

`verify(): Promise<ParsedAttestation>` — throws on failure.

### `verifyAttestation(document, options?)`

| Parameter | Type | Description |
|-----------|------|-------------|
| `document` | `Uint8Array \| Buffer` | Raw binary attestation document |
| `options.debug` | `boolean` | Include raw errors in failure responses (default: `false`) |
| `options.trustedRoot` | `string \| Uint8Array` | Override the trusted root CA certificate |
| `options.nonce` | `Uint8Array \| Buffer` | Require document nonce to equal these bytes |
| `options.maxAgeMs` | `number` | Reject if document timestamp is older than this many ms |
| `options.expectedPcrs` | `Record<string, string>` | Require listed PCR keys to match hex values |

Returns `Promise<AttestationResult>`:

```ts
{ valid: true; document: ParsedAttestation }
{ valid: false; reason: string; error?: unknown }
```

### `verifyTlsLeafBindsAttestation(document, tlsLeafCertificateDer)`

Returns `Promise<boolean>`: `true` if `document.public_key` is 32 bytes and equals `SHA-256(tlsLeafCertificateDer)`.

### `ParsedAttestation`

| Field | Type | Description |
|-------|------|-------------|
| `module_id` | `string` | Enclave module ID |
| `timestamp` | `number` | Document creation time (ms since UNIX epoch) |
| `digest` | `string` | Digest algorithm (`"SHA384"`) |
| `pcrs` | `Record<string, string>` | Hex-encoded PCR values (`pcr0`–`pcr15`, keyed by index) |
| `public_key` | `Uint8Array \| null \| undefined` | Optional binding bytes (Nitrum: SHA-256 of leaf TLS cert DER) |
| `nonce` | `Uint8Array \| null \| undefined` | Optional nonce |
| `user_data` | `Uint8Array \| null \| undefined` | Optional opaque user data |

### Exported types

`AttestationDocument`, `ParsedAttestation`, `AttestationResult`, `AttestationSuccess`, `AttestationFailure`, `VerifyOptions`, `PCRMap`, and `NitrumVerifierInit` (alias for `NitrumVerifierOptions`).

### `AWS_NITRO_ROOT_CA`

PEM string for the official AWS Nitro root CA (default for attestation verification).

## PCR reference

| PCR | Contents |
|-----|----------|
| PCR0 | Enclave image file (EIF) measurement |
| PCR1 | Linux kernel + bootstrap |
| PCR2 | Application |
| PCR3 | IAM role assigned to the parent EC2 instance |
| PCR4 | Instance ID of the parent EC2 instance |
| PCR8 | Signing certificate (if EIF was signed) |

## More documentation

- High‑level Nitrum architecture and how attestation is used end‑to‑end: see `docs/architecture.md` at the root of the Nitrum repository.
- CLI and enclave workflow documentation: see `docs/usage.md`.

## License

MIT
