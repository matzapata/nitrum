import { verifyAttestation, verifyTlsLeafBindsAttestation } from "./attest-verify";
import type { ParsedAttestation } from "./types";
import { base64ToBytes, bytesToBase64, randomBytes } from "./utils";

const DEFAULT_MAX_AGE_MS = 5 * 60_000;

export interface NitrumVerifierOptions {
  /** Enclave HTTPS origin, e.g. `https://nitrum.io` */
  baseUrl: string;
  /**
   * When set, listed PCRs (e.g. `{ pcr0: "abc..." }`) must match the attestation exactly
   * (hex, case-insensitive). Omitted slots are not checked.
   */
  expectedPcrs?: Record<string, string>;
  /**
   * Freshness bound for the attestation document timestamp, in milliseconds.
   * @default 5 * 60_000 (5 minutes)
   */
  maxAgeMs?: number;
  /**
   * Controls Node TLS certificate verification when fetching the leaf certificate for
   * `verifyTlsLeafBindsAttestation`.
   *
   * - `true`  (default): enforce normal TLS verification.
   * - `false`: allow self-signed / invalid certs (for debugging only).
   */
  tlsRejectUnauthorized?: boolean;
  /**
   * Override the trusted root CA certificate (PEM or DER).
   * Defaults to the official AWS Nitro root CA.
   */
  trustedRoot?: string | Uint8Array;
  /**
   * When `true`, the raw error causing validation failure is included in
   * the returned {@link AttestationFailure} object.
   * @default false
   */
  debug?: boolean;
}


export class NitrumVerifier {
  constructor(private readonly init: NitrumVerifierOptions) {
    if (!init.baseUrl?.trim()) {
      throw new Error("NitrumVerifier: baseUrl is required");
    }
  }

  /**
   * GET `/.well-known/enclave/attestation` with a fresh nonce, verify signature/chain/nonce,
   * optional `expectedPcrs`, and always check TLS leaf vs `public_key` hash.
   */
  async verify(): Promise<ParsedAttestation> {
    const nonce = randomBytes(32);
    const base = normalizeBase(this.init.baseUrl);
    const qp = encodeURIComponent(bytesToBase64(nonce));
    const url = `${base}/.well-known/enclave/attestation?nonce=${qp}`;

    const res = await fetch(url);
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`attestation HTTP ${res.status}: ${text.slice(0, 500)}`);
    }

    const raw = base64ToBytes(parseAttestationResponse(text));
    const result = await verifyAttestation(raw, {
      nonce,
      trustedRoot: this.init.trustedRoot,
      debug: this.init.debug,
      maxAgeMs: this.init.maxAgeMs ?? DEFAULT_MAX_AGE_MS,
      expectedPcrs: this.init.expectedPcrs,
    });

    if (!result.valid) {
      throw new Error(result.reason ?? "verifyAttestation failed");
    }

    const u = resolveHttpsBaseUrl(base);
    const port = u.port ? Number(u.port) : 443;
    const rejectUnauthorized = resolveRejectUnauthorized(this.init.tlsRejectUnauthorized);
    const leafDer = await getTlsLeafDerNode(u.hostname, port, rejectUnauthorized);
    const binds = await verifyTlsLeafBindsAttestation(result.document, leafDer);
    if (!binds) {
      throw new Error(
        "TLS leaf does not match attestation public_key (expected SHA-256(DER(leaf)))",
      );
    }

    return result.document;
  }
}

function normalizeBase(base: string): string {
  return base.replace(/\/$/, "");
}

/** Parse attestation endpoint JSON body and return the base64 document string. */
function parseAttestationResponse(text: string): string {
  let data: { document?: string; error?: string };
  try {
    data = JSON.parse(text) as { document?: string; error?: string };
  } catch {
    throw new Error("attestation: expected JSON");
  }
  if (data.error) {
    throw new Error(data.error);
  }
  if (!data.document) {
    throw new Error("attestation: missing document");
  }
  return data.document;
}

/** Resolve and validate verifier base URL, enforcing HTTPS for TLS binding. */
function resolveHttpsBaseUrl(base: string): URL {
  const u = new URL(base.startsWith("http") ? base : `https://${base}`);
  if (u.protocol !== "https:") {
    throw new Error("NitrumVerifier: baseUrl must use https: for TLS binding");
  }
  return u;
}

/** Compute TLS verification behavior from explicit config or process fallback. */
function resolveRejectUnauthorized(explicit?: boolean): boolean {
  if (explicit !== undefined) {
    return explicit;
  }
  return !(
    typeof process !== "undefined" &&
    process.env != null &&
    process.env.NODE_TLS_REJECT_UNAUTHORIZED === "0"
  );
}

/**
 * Node: read TLS leaf DER with configurable `rejectUnauthorized`.
 */
async function getTlsLeafDerNode(
  hostname: string,
  port: number,
  rejectUnauthorized: boolean,
): Promise<Uint8Array> {
  const tls = await import("node:tls");

  return new Promise((resolvePromise, reject) => {
    const socket = tls.connect(
      {
        host: hostname,
        port,
        servername: hostname,
        rejectUnauthorized,
      },
      () => {
        try {
          const cert = socket.getPeerCertificate();
          socket.end();
          if (!cert?.raw || typeof cert.raw === "string") {
            reject(new Error("TLS: no peer certificate DER"));
            return;
          }
          resolvePromise(new Uint8Array(cert.raw));
        } catch (e) {
          socket.end();
          reject(e);
        }
      },
    );
    socket.on("error", reject);
  });
}
