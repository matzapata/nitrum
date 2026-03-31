# nitrum-node

Verify [AWS Nitro Enclave](https://docs.aws.amazon.com/enclaves/latest/user/nitro-enclaves-concepts.html) attestation documents in TypeScript.

**Node.js-focused** — supports Node.js >= 18 and uses Node's native `crypto` module.

This package is part of the **Nitrum** project, which provides a Rust data-plane, control-plane, and CLI for running applications inside Nitro Enclaves. See the repository’s `README.md` and `docs/architecture.md` for a broader overview of how attestation fits into the system.

## Installation

```bash
npm install nitrum-node
```

## Usage

```ts
import { verifyAttestation } from 'nitrum-node';

// `document` is the raw binary attestation document from the enclave
const result = await verifyAttestation(document);

if (result.valid) {
  const { pcrs, module_id, timestamp } = result.document;

  // Always verify PCRs yourself to confirm this is your enclave image
  if (pcrs['pcr0'] !== EXPECTED_PCR0) {
    throw new Error('Unexpected enclave image');
  }

  console.log('Attestation valid ✓', { module_id, timestamp });
} else {
  console.error('Invalid attestation:', result.reason);
}
```

### With debug mode

Pass `{ debug: true }` to include the underlying error in failures — useful during development.

```ts
const result = await verifyAttestation(document, { debug: true });

if (!result.valid) {
  console.error(result.reason, result.error);
}
```

### Custom trusted root

By default the official [AWS Nitro root CA](https://aws-nitro-enclaves.amazonaws.com/AWS_NitroEnclaves_Root-G1.zip) is used. You can supply your own (PEM string or DER bytes):

```ts
import { verifyAttestation, AWS_NITRO_ROOT_CA } from 'nitrum-node';

const result = await verifyAttestation(document, {
  trustedRoot: myCustomRootPem, // string | Uint8Array
});
```

## What is validated

| # | Check |
|---|-------|
| 1 | CBOR structure can be decoded |
| 2 | COSE_Sign1 signature is valid against the embedded signing certificate |
| 3 | Signing certificate was issued by the CA bundle in the document |
| 4 | CA bundle forms a valid chain up to the trusted root CA |

> **PCR verification is your responsibility.**  
> This library confirms the attestation is cryptographically authentic, but you must check the PCR values match the enclave image(s) you trust.

## API

### `verifyAttestation(document, options?)`

| Parameter | Type | Description |
|-----------|------|-------------|
| `document` | `Uint8Array \| Buffer` | Raw binary attestation document |
| `options.debug` | `boolean` | Include raw errors in failure responses (default: `false`) |
| `options.trustedRoot` | `string \| Uint8Array` | Override the trusted root CA certificate |

Returns `Promise<AttestationResult>` — a discriminated union:

```ts
// Success
{ valid: true; document: ParsedAttestation }

// Failure
{ valid: false; reason: string; error?: unknown }
```

### `ParsedAttestation`

| Field | Type | Description |
|-------|------|-------------|
| `module_id` | `string` | Enclave module ID |
| `timestamp` | `number` | Document creation time (ms since epoch) |
| `digest` | `string` | Digest algorithm (`"SHA384"`) |
| `pcrs` | `Record<string, string>` | Hex-encoded PCR values (`pcr0`–`pcr15`) |
| `public_key` | `Uint8Array \| null` | Optional requester-supplied public key |
| `nonce` | `Uint8Array \| null` | Optional nonce for replay protection |
| `user_data` | `Uint8Array \| null` | Optional opaque user data |

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