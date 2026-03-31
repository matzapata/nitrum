/**
 * @file index.ts
 * @description Verify AWS Nitro Enclave attestation documents.
 *
 * Web-compatible (uses SubtleCrypto), zero Node.js-only dependencies at runtime.
 * Works in browsers, Node.js >= 18, Cloudflare Workers, and Deno.
 */

import { Encoder, decode as cborDecode } from 'cbor-x';
import { X509Certificate } from '@peculiar/x509';
import { Crypto as PeculiarCrypto } from '@peculiar/webcrypto';
import { AWS_NITRO_ROOT_CA } from './caroot';
import type {
  AttestationDocument,
  AttestationResult,
  ParsedAttestation,
  VerifyOptions,
} from './types';

/**
 * Default `cbor-x` `encode` tags Uint8Array with CBOR tag 64; COSE Sig_Structure
 * must use untagged byte strings (same as AWS / cose-js).
 */
const coseSigStructureEncoder = new Encoder({ useRecords: false, tagUint8Array: false });

// ---------------------------------------------------------------------------
// Crypto provider — prefer native SubtleCrypto in browsers / modern Node.js
// ---------------------------------------------------------------------------

function getCrypto(): Crypto {
  if (typeof globalThis.crypto !== 'undefined' && globalThis.crypto.subtle) {
    return globalThis.crypto as Crypto;
  }
  return new PeculiarCrypto() as unknown as Crypto;
}

// ---------------------------------------------------------------------------
// Utility: coerce Uint8Array to a plain ArrayBuffer
// Needed because TypeScript 5 distinguishes Uint8Array<ArrayBufferLike>
// from ArrayBufferView<ArrayBuffer> for SubtleCrypto / x509 APIs.
// ---------------------------------------------------------------------------

function toArrayBuffer(u: Uint8Array | Buffer): ArrayBuffer {
  if (u.buffer instanceof ArrayBuffer && u.byteOffset === 0 && u.byteLength === u.buffer.byteLength) {
    return u.buffer as ArrayBuffer;
  }
  return u.buffer.slice(u.byteOffset, u.byteOffset + u.byteLength) as ArrayBuffer;
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/**
 * Verify that `child` was signed by `issuer`.
 */
async function verifyCertSignature(
  child: X509Certificate,
  issuer: X509Certificate,
): Promise<boolean> {
  try {
    return await child.verify({ signatureOnly: true, publicKey: issuer });
  } catch {
    return false;
  }
}

/**
 * Decode the outer COSE_Sign1 structure from a raw attestation document.
 *
 * COSE_Sign1 = [protected-header, unprotected-header, payload, signature]
 */
function decodeCOSESign1(raw: Uint8Array): {
  protectedHeader: Uint8Array;
  payload: Uint8Array;
  signature: Uint8Array;
} {
  const decoded = cborDecode(raw) as unknown;

  let arr: unknown[];
  if (
    decoded !== null &&
    typeof decoded === 'object' &&
    'tag' in (decoded as Record<string, unknown>) &&
    'value' in (decoded as Record<string, unknown>)
  ) {
    arr = (decoded as { value: unknown[] }).value;
  } else if (Array.isArray(decoded)) {
    arr = decoded as unknown[];
  } else {
    throw new Error('Unexpected CBOR structure: expected array or tagged value');
  }

  if (arr.length !== 4) {
    throw new Error(`COSE_Sign1 must have 4 elements, got ${arr.length}`);
  }

  const [protectedHeader, , payload, signature] = arr;

  if (!(protectedHeader instanceof Uint8Array)) throw new Error('protectedHeader is not bytes');
  if (!(payload instanceof Uint8Array)) throw new Error('payload is not bytes');
  if (!(signature instanceof Uint8Array)) throw new Error('signature is not bytes');

  return { protectedHeader, payload, signature };
}

/**
 * Parse the CBOR-encoded attestation document payload.
 */
function parseAttestationPayload(payload: Uint8Array): AttestationDocument {
  const doc = cborDecode(payload) as Record<string, unknown>;

  const required = ['module_id', 'timestamp', 'digest', 'pcrs', 'certificate', 'cabundle'];
  for (const field of required) {
    if (!(field in doc)) throw new Error(`Missing required field: ${field}`);
  }

  const pcrsRaw = doc['pcrs'] as Map<number, Uint8Array> | Record<number, Uint8Array>;
  const pcrs: Map<number, Buffer> = new Map();

  if (pcrsRaw instanceof Map) {
    for (const [k, v] of pcrsRaw) {
      pcrs.set(Number(k), Buffer.from(v));
    }
  } else {
    for (const [k, v] of Object.entries(pcrsRaw)) {
      pcrs.set(Number(k), Buffer.from(v as Uint8Array));
    }
  }

  const ts = doc['timestamp'];
  const timestamp =
    typeof ts === 'bigint' ? Number(ts) : typeof ts === 'number' ? ts : Number(ts);

  return {
    module_id: doc['module_id'] as string,
    timestamp,
    digest: doc['digest'] as string,
    pcrs,
    certificate: doc['certificate'] as Uint8Array,
    cabundle: doc['cabundle'] as Uint8Array[],
    public_key: (doc['public_key'] as Uint8Array | null | undefined) ?? null,
    nonce: (doc['nonce'] as Uint8Array | null | undefined) ?? null,
    user_data: (doc['user_data'] as Uint8Array | null | undefined) ?? null,
  };
}

/**
 * Reconstruct the COSE_Sign1 Sig_Structure for verification.
 *
 * Sig_Structure = ["Signature1", body_protected, external_aad, payload]
 * Matches {@link https://github.com/erdtman/cose-js/blob/master/lib/sign.js cose-js} / common Nitro tooling.
 */
function buildSigStructure(protectedHeader: Uint8Array, payload: Uint8Array): Uint8Array {
  return coseSigStructureEncoder.encode([
    'Signature1',
    protectedHeader,
    new Uint8Array(0),
    payload,
  ]) as Uint8Array;
}

/**
 * Verify the ECDSA-P384 signature over the Sig_Structure using SubtleCrypto.
 */
async function verifySignature(
  sigStructure: Uint8Array,
  signature: Uint8Array,
  publicKeyDer: ArrayBuffer,
  subtleCrypto: SubtleCrypto,
): Promise<boolean> {
  const key = await subtleCrypto.importKey(
    'spki',
    publicKeyDer,
    { name: 'ECDSA', namedCurve: 'P-384' },
    false,
    ['verify'],
  );

  return subtleCrypto.verify(
    { name: 'ECDSA', hash: 'SHA-384' },
    key,
    toArrayBuffer(signature),
    toArrayBuffer(sigStructure),
  );
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Verify an AWS Nitro Enclave attestation document.
 *
 * Validates:
 * 1. CBOR structure integrity
 * 2. COSE_Sign1 signature against the embedded certificate
 * 3. Certificate chain from the signing cert up to the AWS Nitro root CA
 * 4. Optional: `options.nonce` must match the document’s embedded nonce (replay binding)
 *
 * **You are still responsible** for checking the PCR values to confirm the
 * attestation was produced by the specific enclave image you trust.
 *
 * @example
 * ```ts
 * import { verifyAttestation } from 'nitrum';
 *
 * const result = await verifyAttestation(documentBuffer);
 *
 * if (result.valid) {
 *   console.log('PCR0:', result.document.pcrs['pcr0']);
 * } else {
 *   console.error('Invalid attestation:', result.reason);
 * }
 * ```
 *
 * @param document - Raw attestation document bytes (binary CBOR)
 * @param options  - Verification options
 */
function toUint8Array(u: Uint8Array | Buffer): Uint8Array {
  return u instanceof Uint8Array ? u : Uint8Array.from(u);
}

export async function verifyAttestation(
  document: Uint8Array | Buffer,
  options: VerifyOptions = {},
): Promise<AttestationResult> {
  const { debug = false, trustedRoot = AWS_NITRO_ROOT_CA, nonce: expectedNonce } = options;
  const subtle = getCrypto().subtle;

  // Step 1: Decode the outer COSE_Sign1 wrapper

  let protectedHeader: Uint8Array;
  let payload: Uint8Array;
  let signature: Uint8Array;

  try {
    ({ protectedHeader, payload, signature } = decodeCOSESign1(document as Uint8Array));
  } catch (error) {
    return debug
      ? { valid: false, reason: 'Failed to decode COSE_Sign1 structure', error }
      : { valid: false, reason: 'Failed to decode attestation document' };
  }

  // Step 2: Decode the attestation document payload

  let attestation: AttestationDocument;

  try {
    attestation = parseAttestationPayload(payload);
  } catch (error) {
    return debug
      ? { valid: false, reason: 'Failed to parse attestation document payload', error }
      : { valid: false, reason: 'Failed to parse attestation document' };
  }

  // Step 3: Verify the COSE signature

  try {
    const cert = new X509Certificate(toArrayBuffer(attestation.certificate as Uint8Array));
    const publicKeyDer = cert.publicKey.rawData as ArrayBuffer;
    const sigStructure = buildSigStructure(protectedHeader, payload);
    const valid = await verifySignature(sigStructure, signature, publicKeyDer, subtle);

    if (!valid) {
      return { valid: false, reason: 'Attestation document signature is invalid' };
    }
  } catch (error) {
    return debug
      ? { valid: false, reason: 'Failed to verify attestation document signature', error }
      : { valid: false, reason: 'Failed to verify attestation document signature' };
  }


  // Step 4: Validate the certificate chain

  try {
    const signingCert = new X509Certificate(toArrayBuffer(attestation.certificate as Uint8Array));
    const intermediates = (attestation.cabundle as Uint8Array[]).map(
      (der) => new X509Certificate(toArrayBuffer(der)),
    );
    const rootInput: string | ArrayBuffer =
      trustedRoot instanceof Uint8Array ? toArrayBuffer(trustedRoot) : trustedRoot;
    const rootCert = new X509Certificate(rootInput);

    const topIntermediate = intermediates[intermediates.length - 1];
    if (!topIntermediate) {
      return { valid: false, reason: 'Certificate bundle is empty' };
    }

    if (!(await verifyCertSignature(signingCert, topIntermediate))) {
      return { valid: false, reason: 'Signing certificate was not issued by the CA bundle' };
    }

    for (let i = intermediates.length - 1; i > 0; i--) {
      if (!(await verifyCertSignature(intermediates[i]!, intermediates[i - 1]!))) {
        return { valid: false, reason: 'Certificate chain validation failed' };
      }
    }

    if (!(await verifyCertSignature(intermediates[0]!, rootCert))) {
      return { valid: false, reason: 'Certificate chain does not anchor to the trusted root CA' };
    }
  } catch (error) {
    return debug
      ? { valid: false, reason: 'Certificate chain validation error', error }
      : { valid: false, reason: 'Failed to validate certificate chain' };
  }

  // Step 5: Optional nonce binding (same bytes as passed into attestation)

  if (expectedNonce !== undefined) {
    const expected = toUint8Array(expectedNonce);
    const actual = attestation.nonce;
    if (actual == null) {
      return { valid: false, reason: 'Attestation document has no nonce' };
    }
    if (!timingSafeEqualUint8(toUint8Array(actual), expected)) {
      return { valid: false, reason: 'Attestation nonce does not match expected value' };
    }
  }

  // Step 6: Build the result

  const pcrs: Record<string, string> = {};
  attestation.pcrs.forEach((value, index) => {
    pcrs[`pcr${index}`] = value.toString('hex');
  });

  const documentOut: ParsedAttestation = {
    module_id: attestation.module_id,
    timestamp: attestation.timestamp,
    digest: attestation.digest,
    pcrs,
    public_key: attestation.public_key,
    nonce: attestation.nonce,
    user_data: attestation.user_data,
  };

  return { valid: true, document: documentOut };
}

/** Constant-time comparison of equal-length byte strings. */
function timingSafeEqualUint8(a: Uint8Array, b: Uint8Array): boolean {
  if (a.byteLength !== b.byteLength) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.byteLength; i++) {
    diff |= a[i]! ^ b[i]!;
  }
  return diff === 0;
}

/**
 * Check that a TLS **leaf** certificate matches the attestation `public_key` binding.
 *
 * **Not the X.509 in `document.certificate`:** that value is the Nitro **attestation signing**
 * certificate (AWS PKI) used for the COSE signature — it is unrelated to HTTPS.
 *
 * **Nitrum data-plane** passes `SHA-256(DER(leaf TLS certificate))` into NSM as the
 * `public_key` parameter, so honest enclaves embed a 32-byte hash verifiers can compare to
 * the certificate presented on the TLS connection.
 *
 * Other callers (e.g. KMS recipient flows) may put a full SPKI in `public_key`; those are
 * not 32 bytes, so this function returns `false` and you must use a different check.
 *
 * @param document - Parsed attestation (needs `public_key` from {@link verifyAttestation})
 * @param tlsLeafCertificateDer - DER encoding of the **end-entity** cert the client saw on the wire
 */
export async function verifyTlsLeafBindsAttestation(
  document: Pick<ParsedAttestation, 'public_key'>,
  tlsLeafCertificateDer: Uint8Array | Buffer,
): Promise<boolean> {
  const bound = document.public_key;
  if (!bound || bound.byteLength !== 32) {
    return false;
  }
  const subtle = getCrypto().subtle;
  const digest = await subtle.digest(
    'SHA-256',
    toArrayBuffer(tlsLeafCertificateDer as Uint8Array),
  );
  const expected = bound instanceof Uint8Array ? bound : Uint8Array.from(bound);
  return timingSafeEqualUint8(new Uint8Array(digest), expected);
}
