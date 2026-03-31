/**
 * Core attestation verification (COSE, chain, optional nonce / PCR checks).
 */

import { createHash, createVerify } from "node:crypto";
import { Encoder, decode as cborDecode } from "cbor-x";
import { X509Certificate } from "@peculiar/x509";
import { AWS_NITRO_ROOT_CA } from "./caroot";
import type {
  AttestationDocument,
  AttestationResult,
  ParsedAttestation,
  VerifyOptions,
} from "./types";
import { toArrayBuffer } from "./utils";


/**
 * Verify an AWS Nitro Enclave attestation document.
 *
 * Performs:
 * - COSE_Sign1 structure decoding
 * - Payload parsing and shape validation
 * - Signature verification against the embedded certificate
 * - Certificate chain validation up to the trusted root
 * - Optional nonce, maxAgeMs, and expectedPcrs checks
 */
export async function verifyAttestation(
  document: Uint8Array | Buffer,
  options: VerifyOptions = {},
): Promise<AttestationResult> {
  const {
    debug = false,
    trustedRoot = AWS_NITRO_ROOT_CA,
    nonce: expectedNonce,
    maxAgeMs,
    expectedPcrs,
  } = options;

  let sign1: DecodedSign1;

  try {
    sign1 = decodeCOSESign1(document as Uint8Array);
  } catch (error) {
    return debug
      ? { valid: false, reason: "Failed to decode COSE_Sign1 structure", error }
      : { valid: false, reason: "Failed to decode attestation document" };
  }

  let attestation: AttestationDocument;

  try {
    attestation = parseAttestationPayload(sign1.payload);
  } catch (error) {
    return debug
      ? { valid: false, reason: "Failed to parse attestation document payload", error }
      : { valid: false, reason: "Failed to parse attestation document" };
  }

  try {
    const signatureErr = verifyCoseSignature(sign1, attestation);
    if (signatureErr) {
      return signatureErr;
    }
  } catch (error) {
    return debug
      ? { valid: false, reason: "Failed to verify attestation document signature", error }
      : { valid: false, reason: "Failed to verify attestation document signature" };
  }

  try {
    const chainErr = await verifyAttestationCertificateChain(attestation, trustedRoot);
    if (chainErr) {
      return chainErr;
    }
  } catch (error) {
    return debug
      ? { valid: false, reason: "Certificate chain validation error", error }
      : { valid: false, reason: "Failed to validate certificate chain" };
  }

  const policyErr = verifyPolicyConstraints(attestation, expectedNonce, maxAgeMs);
  if (policyErr) {
    return policyErr;
  }

  const documentOut = toParsedAttestation(attestation);

  if (expectedPcrs && Object.keys(expectedPcrs).length > 0) {
    const pcrErr = checkExpectedPcrs(documentOut, expectedPcrs);
    if (pcrErr) {
      return { valid: false, reason: pcrErr };
    }
  }

  return { valid: true, document: documentOut };
}

/**
 * Verify that the attestation `public_key` binding matches the SHA-256 hash
 * of the TLS leaf certificate DER.
 */
export async function verifyTlsLeafBindsAttestation(
  document: Pick<ParsedAttestation, "public_key">,
  tlsLeafCertificateDer: Uint8Array | Buffer,
): Promise<boolean> {
  const bound = document.public_key;
  if (!bound || bound.byteLength !== 32) {
    return false;
  }
  const digest = createHash("sha256")
    .update(Buffer.from(tlsLeafCertificateDer))
    .digest();
  const expected = bound instanceof Uint8Array ? bound : Uint8Array.from(bound);
  return timingSafeEqualUint8(new Uint8Array(digest), expected);
}



/** Minimal decoded COSE_Sign1 parts needed by verification. */
interface DecodedSign1 {
  protectedHeader: Uint8Array;
  payload: Uint8Array;
  signature: Uint8Array;
}

/** Verify that `child` was signed by `issuer` using x509 APIs. */
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
 * Decode a CBOR-encoded COSE_Sign1 structure and extract the key components.
 */
function decodeCOSESign1(raw: Uint8Array): DecodedSign1 {
  const decoded = cborDecode(raw) as unknown;

  let arr: unknown[];
  if (
    decoded !== null &&
    typeof decoded === "object" &&
    "tag" in (decoded as Record<string, unknown>) &&
    "value" in (decoded as Record<string, unknown>)
  ) {
    arr = (decoded as { value: unknown[] }).value;
  } else if (Array.isArray(decoded)) {
    arr = decoded as unknown[];
  } else {
    throw new Error("Unexpected CBOR structure: expected array or tagged value");
  }

  if (arr.length !== 4) {
    throw new Error(`COSE_Sign1 must have 4 elements, got ${arr.length}`);
  }

  const [protectedHeader, , payload, signature] = arr;

  if (!(protectedHeader instanceof Uint8Array)) throw new Error("protectedHeader is not bytes");
  if (!(payload instanceof Uint8Array)) throw new Error("payload is not bytes");
  if (!(signature instanceof Uint8Array)) throw new Error("signature is not bytes");

  return { protectedHeader, payload, signature };
}

/**
 * Parse the attestation document payload (CBOR) into a typed structure.
 */
function parseAttestationPayload(payload: Uint8Array): AttestationDocument {
  const doc = cborDecode(payload) as Record<string, unknown>;

  const required = ["module_id", "timestamp", "digest", "pcrs", "certificate", "cabundle"];
  for (const field of required) {
    if (!(field in doc)) throw new Error(`Missing required field: ${field}`);
  }

  const pcrsRaw = doc.pcrs as Map<number, Uint8Array> | Record<number, Uint8Array>;
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

  const ts = doc.timestamp;
  const timestamp = typeof ts === "bigint" ? Number(ts) : typeof ts === "number" ? ts : Number(ts);

  return {
    module_id: doc.module_id as string,
    timestamp,
    digest: doc.digest as string,
    pcrs,
    certificate: doc.certificate as Uint8Array,
    cabundle: doc.cabundle as Uint8Array[],
    public_key: (doc.public_key as Uint8Array | null | undefined) ?? null,
    nonce: (doc.nonce as Uint8Array | null | undefined) ?? null,
    user_data: (doc.user_data as Uint8Array | null | undefined) ?? null,
  };
}

/**
 * Build the COSE Sig_Structure array used for ECDSA signature verification.
 */
function buildSigStructure(protectedHeader: Uint8Array, payload: Uint8Array): Uint8Array {
  const coseSigStructureEncoder = new Encoder({ useRecords: false, tagUint8Array: false });
  return coseSigStructureEncoder.encode([
    "Signature1",
    protectedHeader,
    new Uint8Array(0),
    payload,
  ]) as Uint8Array;
}

/**
 * Verify an ECDSA-P384 signature over a Sig_Structure using Node's `crypto` module.
 */
function verifySignature(
  sigStructure: Uint8Array,
  signature: Uint8Array,
  publicKeyDer: ArrayBuffer,
): boolean {
  const verifier = createVerify("sha384");
  verifier.update(Buffer.from(sigStructure));
  verifier.end();
  const derSig = ecdsaP384RawToDer(signature);
  return verifier.verify(
    {
      key: Buffer.from(publicKeyDer),
      format: "der",
      type: "spki",
    },
    derSig,
  );
}

/** Normalize a `Uint8Array` or `Buffer` into a `Uint8Array` view. */
function toUint8Array(u: Uint8Array | Buffer): Uint8Array {
  return u instanceof Uint8Array ? u : Uint8Array.from(u);
}

/** Constant-time comparison of equal-length byte strings. */
function timingSafeEqualUint8(a: Uint8Array, b: Uint8Array): boolean {
  if (a.byteLength !== b.byteLength) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.byteLength; i++) {
    diff |= a[i] ^ b[i];
  }
  return diff === 0;
}

/** Decode a hex string (optionally `0x`-prefixed) into raw bytes. */
function hexToBytes(hex: string): Uint8Array | null {
  const h = hex.replace(/^0x/i, "").toLowerCase();
  if (h.length % 2 !== 0) return null;
  const out = new Uint8Array(h.length / 2);
  for (let i = 0; i < out.length; i++) {
    const byte = parseInt(h.slice(i * 2, i * 2 + 2), 16);
    if (Number.isNaN(byte)) return null;
    out[i] = byte;
  }
  return out;
}

/**
 * Convert a raw ECDSA P-384 signature (r||s, 96 bytes) into DER-encoded form.
 */
function ecdsaP384RawToDer(raw: Uint8Array): Buffer {
  const P384_ECDSA_RAW_SIG_LEN = 96;
  if (raw.byteLength !== P384_ECDSA_RAW_SIG_LEN) {
    throw new Error(`Expected ${P384_ECDSA_RAW_SIG_LEN}-byte raw P-384 signature`);
  }
  const r = Buffer.from(raw.subarray(0, 48));
  const s = Buffer.from(raw.subarray(48));

  const trimLeadingZeros = (buf: Buffer): Buffer => {
    let i = 0;
    while (i < buf.length - 1 && buf[i] === 0) {
      i++;
    }
    let v = buf.subarray(i);
    // If high bit is set, prefix with 0x00 to keep it positive.
    if (v[0] & 0x80) {
      v = Buffer.concat([Buffer.from([0]), v]);
    }
    return v;
  };

  const rInt = trimLeadingZeros(r);
  const sInt = trimLeadingZeros(s);

  const len = 2 + rInt.length + 2 + sInt.length;
  return Buffer.concat([
    Buffer.from([0x30, len]),
    Buffer.from([0x02, rInt.length]),
    rInt,
    Buffer.from([0x02, sInt.length]),
    sInt,
  ]);
}

/**
 * When `expected` is non-empty, every listed key (e.g. pcr0) must exist on the document
 * and match the expected hex (case-insensitive).
 */
function checkExpectedPcrs(
  documentOut: ParsedAttestation,
  expected: Record<string, string>,
): string | null {
  for (const key of Object.keys(expected)) {
    const actualHex = documentOut.pcrs[key];
    if (actualHex === undefined) {
      return `Missing PCR ${key} in attestation`;
    }
    const actualBytes = hexToBytes(actualHex);
    const expectedValue = expected[key];
    const expectedBytes = expectedValue != null ? hexToBytes(expectedValue) : null;
    if (actualBytes === null || expectedBytes === null) {
      return `Invalid hex for PCR ${key}`;
    }
    if (actualBytes.byteLength !== expectedBytes.byteLength) {
      return `PCR ${key} length mismatch`;
    }
    if (!timingSafeEqualUint8(actualBytes, expectedBytes)) {
      return `PCR ${key} does not match expected value`;
    }
  }
  return null;
}

/** Verify the COSE signature against the certificate embedded in attestation payload. */
function verifyCoseSignature(
  sign1: DecodedSign1,
  attestation: AttestationDocument,
): AttestationResult | null {
  const cert = new X509Certificate(toArrayBuffer(attestation.certificate as Uint8Array));
  const publicKeyDer = cert.publicKey.rawData as ArrayBuffer;
  const sigStructure = buildSigStructure(sign1.protectedHeader, sign1.payload);
  const valid = verifySignature(sigStructure, sign1.signature, publicKeyDer);
  return valid ? null : { valid: false, reason: "Attestation document signature is invalid" };
}

/** Verify certificate chain from signing cert to the trusted Nitro root. */
async function verifyAttestationCertificateChain(
  attestation: AttestationDocument,
  trustedRoot: string | Uint8Array,
): Promise<AttestationResult | null> {
  const signingCert = new X509Certificate(toArrayBuffer(attestation.certificate as Uint8Array));
  const intermediates = (attestation.cabundle as Uint8Array[]).map(
    (der) => new X509Certificate(toArrayBuffer(der)),
  );
  const rootInput: string | ArrayBuffer =
    trustedRoot instanceof Uint8Array ? toArrayBuffer(trustedRoot) : trustedRoot;
  const rootCert = new X509Certificate(rootInput);

  const topIntermediate = intermediates[intermediates.length - 1];
  if (!topIntermediate) {
    return { valid: false, reason: "Certificate bundle is empty" };
  }

  if (!(await verifyCertSignature(signingCert, topIntermediate))) {
    return { valid: false, reason: "Signing certificate was not issued by the CA bundle" };
  }

  for (let i = intermediates.length - 1; i > 0; i--) {
    const current = intermediates[i];
    const previous = intermediates[i - 1];
    if (!current || !previous) {
      return { valid: false, reason: "Certificate chain validation failed" };
    }
    if (!(await verifyCertSignature(current, previous))) {
      return { valid: false, reason: "Certificate chain validation failed" };
    }
  }

  const firstIntermediate = intermediates[0];
  if (!firstIntermediate) {
    return { valid: false, reason: "Certificate chain validation failed" };
  }
  if (!(await verifyCertSignature(firstIntermediate, rootCert))) {
    return { valid: false, reason: "Certificate chain does not anchor to the trusted root CA" };
  }

  return null;
}

/** Check caller-provided freshness and nonce policy constraints. */
function verifyPolicyConstraints(
  attestation: AttestationDocument,
  expectedNonce: Uint8Array | Buffer | undefined,
  maxAgeMs: number | undefined,
): AttestationResult | null {
  if (expectedNonce !== undefined) {
    const expected = toUint8Array(expectedNonce);
    const actual = attestation.nonce;
    if (actual == null) {
      return { valid: false, reason: "Attestation document has no nonce" };
    }
    if (!timingSafeEqualUint8(toUint8Array(actual), expected)) {
      return { valid: false, reason: "Attestation nonce does not match expected value" };
    }
  }

  if (typeof maxAgeMs === "number") {
    if (!(maxAgeMs > 0)) {
      return { valid: false, reason: "maxAgeMs must be a positive number" };
    }
    const age = Date.now() - attestation.timestamp;
    if (age > maxAgeMs) {
      return { valid: false, reason: "Attestation document is too old" };
    }
  }

  return null;
}

/** Convert a verified attestation payload to public output shape. */
function toParsedAttestation(attestation: AttestationDocument): ParsedAttestation {
  const pcrs: Record<string, string> = {};
  attestation.pcrs.forEach((value, index) => {
    pcrs[`pcr${index}`] = value.toString("hex");
  });

  return {
    module_id: attestation.module_id,
    timestamp: attestation.timestamp,
    digest: attestation.digest,
    pcrs,
    public_key: attestation.public_key,
    nonce: attestation.nonce,
    user_data: attestation.user_data,
  };
}
