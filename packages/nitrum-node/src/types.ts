/**
 * A single PCR (Platform Configuration Register) value.
 * Index maps to the PCR slot number (0–15).
 */
export type PCRMap = Map<number, Buffer>;

/**
 * Decoded AWS Nitro attestation document fields.
 * See: https://docs.aws.amazon.com/enclaves/latest/user/verify-root.html
 */
export interface AttestationDocument {
  /** DER-encoded X.509 signing certificate */
  certificate: Uint8Array;
  /** CA certificate chain from signing cert up to (but not including) root */
  cabundle: Uint8Array[];
  /** Module ID of the Nitro enclave */
  module_id: string;
  /** UNIX timestamp (milliseconds) when the document was created */
  timestamp: number;
  /** Digest algorithm used — always "SHA384" for Nitro */
  digest: string;
  /** Platform Configuration Registers as raw bytes */
  pcrs: PCRMap;
  /**
   * Optional opaque binding bytes passed into NSM as `public_key`.
   * Nitrum’s ingress uses **`SHA-256(DER(leaf TLS certificate))`** (32 bytes) so verifiers
   * can tie the document to the HTTPS certificate; other stacks may embed SPKI or other data.
   */
  public_key?: Uint8Array | null;
  /** Optional user-supplied nonce for replay protection */
  nonce?: Uint8Array | null;
  /** Optional opaque user data */
  user_data?: Uint8Array | null;
}

/** Parsed, validated attestation document with hex-encoded PCRs */
export interface ParsedAttestation
  extends Omit<AttestationDocument, "pcrs" | "certificate" | "cabundle"> {
  /** Hex-encoded PCR values, keyed by PCR index (e.g. "pcr0", "pcr1", ...) */
  pcrs: Record<string, string>;
}

/** Returned by {@link verifyAttestation} on success */
export interface AttestationSuccess {
  valid: true;
  /** Fully validated attestation document attributes */
  document: ParsedAttestation;
}

/** Returned by {@link verifyAttestation} on failure */
export interface AttestationFailure {
  valid: false;
  /** Human-readable reason for validation failure */
  reason: string;
  /** Original error, only present when `debug` is `true` */
  error?: unknown;
}

export type AttestationResult = AttestationSuccess | AttestationFailure;

/** Options accepted by {@link verifyAttestation} */
export interface VerifyOptions {
  /**
   * When `true`, the raw error causing validation failure is included in
   * the returned {@link AttestationFailure} object.
   * @default false
   */
  debug?: boolean;
  /**
   * Override the trusted root CA certificate (PEM or DER).
   * Defaults to the official AWS Nitro root CA.
   */
  trustedRoot?: string | Uint8Array;
  /**
   * When set, the signed attestation payload must contain a `nonce` field equal to these
   * bytes (constant-time comparison). Use the same raw bytes you passed to NSM / the
   * Nitrum attestation API, not a base64 string.
   */
  nonce?: Uint8Array | Buffer;
  /**
   * When set, the attestation document timestamp (milliseconds since UNIX epoch) must not
   * be older than this many milliseconds relative to the verifier's current time.
   * This is an additional freshness bound on top of nonce-based replay protection.
   */
  maxAgeMs?: number;
  /**
   * When set, each key (e.g. `pcr0`, `pcr1`) must be present on the document and match the
   * given hex string (case-insensitive). Unlisted PCRs are not checked.
   */
  expectedPcrs?: Record<string, string>;
}
