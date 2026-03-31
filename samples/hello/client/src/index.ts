/**
 * Exercise the hello sample against:
 * - Data-plane well-known: GET /.well-known/enclave/{status,attestation}
 * - Enclave app (Express): routes from enclave/src/main.js
 *
 * POST /attestation on the app proxies to the Nitrum crypto API and returns
 * `{ document: "<base64>" }`. Decode and verify with nitro-attestation.
 */
import { randomBytes } from 'node:crypto';
import { resolve } from 'node:path';
import tls from 'node:tls';
import { fileURLToPath } from 'node:url';
import { verifyAttestation, verifyTlsLeafBindsAttestation } from 'nitro-attestation';

function nonceBytesToHex(n: Uint8Array | Buffer | null | undefined): string | null {
  if (n == null || n.byteLength === 0) {
    return null;
  }
  return Buffer.from(n).toString('hex');
}

function enclaveHostLooksLikeAwsElb(url: string): boolean {
  try {
    const u = new URL(url);
    return u.protocol === 'https:' && /\.elb\.[^.]+\.amazonaws\.com$/i.test(u.hostname);
  } catch {
    return false;
  }
}

const baseUrl = process.env.ENCLAVE_URL as string;
if (!baseUrl) {
  throw new Error('ENCLAVE_URL is not set');
}

/** Self-signed or private CA, or Nitrum ELB sample hosts (override with ENCLAVE_TLS_STRICT=1). */
const tlsInsecureExplicit =
  process.env.ENCLAVE_TLS_INSECURE === '1' || process.env.ENCLAVE_TLS_INSECURE === 'true';
const tlsStrict = process.env.ENCLAVE_TLS_STRICT === '1' || process.env.ENCLAVE_TLS_STRICT === 'true';
const tlsInsecure =
  tlsInsecureExplicit || (!tlsStrict && enclaveHostLooksLikeAwsElb(baseUrl));
if (tlsInsecure) {
  process.env.NODE_TLS_REJECT_UNAUTHORIZED = '0';
  console.warn(
    tlsInsecureExplicit
      ? 'ENCLAVE_TLS_INSECURE: TLS certificate verification is disabled for this process.'
      : 'ENCLAVE_URL host looks like an AWS ELB; disabling TLS certificate verification (set ENCLAVE_TLS_STRICT=1 to enforce).',
  );
}

function normalizeBase(base: string): string {
  return base.replace(/\/$/, '');
}

/** DER of the TLS leaf certificate (HTTPS only). Honors NODE_TLS_REJECT_UNAUTHORIZED. */
function getTlsLeafDer(hostname: string, port: number): Promise<Uint8Array> {
  const rejectUnauthorized = process.env.NODE_TLS_REJECT_UNAUTHORIZED !== '0';
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
          if (!cert?.raw || typeof cert.raw === 'string') {
            reject(new Error('TLS: no peer certificate DER'));
            return;
          }
          resolvePromise(new Uint8Array(cert.raw));
        } catch (e) {
          socket.end();
          reject(e);
        }
      },
    );
    socket.on('error', reject);
  });
}

/** Nitrum ingress embeds SHA-256(TLS leaf DER) in `public_key`; the app crypto `/attestation` route does not. */
async function assertAttestationBindsTls(
  base: string,
  document: { public_key?: Uint8Array | null },
): Promise<void> {
  const u = new URL(base.startsWith('http') ? base : `https://${base}`);
  if (u.protocol !== 'https:') {
    return;
  }
  const port = u.port ? Number(u.port) : 443;
  const leafDer = await getTlsLeafDer(u.hostname, port);
  console.log('leafDer', Buffer.from(leafDer).toString('base64'));
  const ok = await verifyTlsLeafBindsAttestation(document, leafDer);
  if (!ok) {
    throw new Error(
      'TLS leaf does not match attestation public_key (expected SHA-256(DER(leaf)))',
    );
  }
}

interface AttestationResponse {
  document?: string;
  error?: string;
}

async function fetchAttestationPost(
  base: string,
): Promise<{ raw: Uint8Array; nonce: Uint8Array }> {
  const nonce = randomBytes(32);
  const res = await fetch(`${normalizeBase(base)}/attestation`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ nonce: nonce.toString('base64') }),
  });
  const text = await res.text();
  console.log('text', text, nonce.toString('base64'));

  if (!res.ok) {
    throw new Error(`attestation HTTP ${res.status}: ${text}`);
  }
  let data: AttestationResponse;
  try {
    data = JSON.parse(text) as AttestationResponse;
  } catch {
    throw new Error(`attestation: expected JSON, got: ${text.slice(0, 200)}`);
  }
  if (data.error) {
    throw new Error(data.error);
  }
  if (!data.document) {
    throw new Error('attestation: missing document in response');
  }
  return {
    raw: Uint8Array.from(Buffer.from(data.document, 'base64')),
    nonce,
  };
}

async function fetchAttestationWellKnown(
  base: string,
): Promise<{ raw: Uint8Array; nonce: Uint8Array }> {
  const nonce = randomBytes(32);
  const qp = encodeURIComponent(nonce.toString('base64'));
  const res = await fetch(
    `${normalizeBase(base)}/.well-known/enclave/attestation?nonce=${qp}`,
  );
  const text = await res.text();
  console.log('text', text, nonce.toString('base64'));
  if (!res.ok) {
    throw new Error(`well-known attestation HTTP ${res.status}: ${text}`);
  }
  let data: AttestationResponse;
  try {
    data = JSON.parse(text) as AttestationResponse;
  } catch {
    throw new Error(`well-known attestation: expected JSON, got: ${text.slice(0, 200)}`);
  }
  if (data.error) {
    throw new Error(data.error);
  }
  if (!data.document) {
    throw new Error('well-known attestation: missing document in response');
  }
  return {
    raw: Uint8Array.from(Buffer.from(data.document, 'base64')),
    nonce,
  };
}

async function runStep<T>(label: string, fn: () => Promise<T>): Promise<boolean> {
  console.log(`\n--- ${label} ---`);
  try {
    const out = await fn();
    if (out !== undefined) {
      console.log(typeof out === 'string' ? out : JSON.stringify(out, null, 2));
    }
    return true;
  } catch (err: unknown) {
    if (err instanceof Error) {
      const cause = err.cause instanceof Error ? ` (${err.cause.message})` : '';
      console.error('Error:', err.message + cause);
    } else {
      console.error('Error:', err);
    }
    return false;
  }
}

export async function probeEnclave(base: string = baseUrl): Promise<boolean> {
  const b = normalizeBase(base);
  let ok = true;

  ok = (await runStep('GET /.well-known/enclave/status', async () => {
    const res = await fetch(`${b}/.well-known/enclave/status`);
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return JSON.parse(text) as unknown;
  })) && ok;
  ok = (await runStep('POST /attestation (app proxy, verify)', async () => {
    const { raw, nonce } = await fetchAttestationPost(b);
    const result = await verifyAttestation(raw, { nonce });
    if (!result.valid) {
      throw new Error(result.reason ?? 'verification failed');
    }
    return {
      valid: true,
      module_id: result.document.module_id,
      timestamp: result.document.timestamp,
      digest: result.document.digest,
      pcrs: result.document.pcrs,
      nonce_hex: nonceBytesToHex(result.document.nonce),
    };
  })) && ok;

  ok = (await runStep('GET /.well-known/enclave/attestation (verify)', async () => {
    const { raw, nonce } = await fetchAttestationWellKnown(b);
    const result = await verifyAttestation(raw, { nonce });
    if (!result.valid) {
      throw new Error(result.reason ?? 'verification failed');
    }
    await assertAttestationBindsTls(b, result.document);
    return {
      valid: true,
      module_id: result.document.module_id,
      timestamp: result.document.timestamp,
      digest: result.document.digest,
      pcrs: result.document.pcrs,
      nonce_hex: nonceBytesToHex(result.document.nonce),
      tls_leaf_binds_attestation: true,
    };
  })) && ok;
  // return true;

  ok = (await runStep('GET /health', async () => {
    const res = await fetch(`${b}/health`);
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return text;
  })) && ok;

  ok = (await runStep('GET /egress', async () => {
    const res = await fetch(`${b}/egress`);
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return JSON.parse(text) as unknown;
  })) && ok;



  ok = (await runStep('POST /crypto', async () => {
    const res = await fetch(`${b}/crypto`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ plaintext: 'hello' }),
    });
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return JSON.parse(text) as unknown;
  })) && ok;

  ok = (await runStep('POST /random', async () => {
    const res = await fetch(`${b}/random`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    });
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return JSON.parse(text) as unknown;
  })) && ok;

  ok = (await runStep('GET /env', async () => {
    const res = await fetch(`${b}/env`);
    const text = await res.text();
    if (!res.ok) {
      throw new Error(`HTTP ${res.status}: ${text}`);
    }
    return JSON.parse(text) as unknown;
  })) && ok;

  return ok;
}

async function main(): Promise<void> {
  console.log('ENCLAVE_URL:', baseUrl);
  const ok = await probeEnclave();
  if (!ok) {
    process.exitCode = 1;
  }
}

if (resolve(fileURLToPath(import.meta.url)) === resolve(process.argv[1] ?? '')) {
  main().catch((err: unknown) => {
    console.error(err);
    process.exitCode = 1;
  });
}
