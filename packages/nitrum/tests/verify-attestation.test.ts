import { createHash, randomBytes } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { verifyAttestation, verifyTlsLeafBindsAttestation } from '../src/index';

const dir = dirname(fileURLToPath(import.meta.url));
const fixtures = join(dir, 'fixtures');

/** Fixture files are base64 text (same shape as API `document` fields), not raw CBOR bytes. */
function loadAttestationFromBase64File(name: string): Uint8Array {
  const text = readFileSync(join(fixtures, name), 'utf8').trim();
  return Uint8Array.from(Buffer.from(text, 'base64'));
}

describe('verifyAttestation', () => {
  it('accepts valid.cbor', async () => {
    const raw = loadAttestationFromBase64File("valid.cbor");
    const result = await verifyAttestation(raw);

    expect(result.valid).toBe(true);
    if (!result.valid) return;
    expect(result.document.module_id).toBeTruthy();
    expect(result.document.digest).toBeTruthy();
    expect(typeof result.document.timestamp).toBe('number');
    expect(Object.keys(result.document.pcrs).length).toBeGreaterThan(0);
  });

  it('accepts debug-mode-document.cbor (still pin PCRs / image policy in production)', async () => {
    const raw = loadAttestationFromBase64File('debug-mode-document.cbor');
    const result = await verifyAttestation(raw);

    expect(result.valid).toBe(true);
    if (!result.valid) return;
    expect(result.document.module_id).toBeTruthy();
  });

  it('rejects invalid-signature.cbor', async () => {
    const raw = loadAttestationFromBase64File('invalid-signature.cbor');
    const result = await verifyAttestation(raw);

    expect(result.valid).toBe(false);
    if (result.valid) return;
    expect(result.reason).toBe('Attestation document signature is invalid');
  });

  it('rejects when options.nonce does not match the document', async () => {
    const raw = loadAttestationFromBase64File('valid.cbor');
    const result = await verifyAttestation(raw, { nonce: randomBytes(32) });

    expect(result.valid).toBe(false);
    if (result.valid) return;
    expect(
      result.reason === 'Attestation document has no nonce' ||
        result.reason === 'Attestation nonce does not match expected value',
    ).toBe(true);
  });

  it('accepts valid.cbor when options.nonce matches embedded nonce', async () => {
    const raw = loadAttestationFromBase64File('valid.cbor');
    const first = await verifyAttestation(raw);
    expect(first.valid).toBe(true);
    if (!first.valid) return;
    const n = first.document.nonce;
    if (!n || n.byteLength === 0) {
      return;
    }
    const nonce = n instanceof Uint8Array ? n : Uint8Array.from(n);
    const second = await verifyAttestation(raw, { nonce });
    expect(second.valid).toBe(true);
  });
});

describe('verifyTlsLeafBindsAttestation', () => {
  it('returns true when SHA-256(DER) matches public_key', async () => {
    const der = randomBytes(100);
    const h = createHash('sha256').update(der).digest();
    await expect(
      verifyTlsLeafBindsAttestation({ public_key: Uint8Array.from(h) }, der),
    ).resolves.toBe(true);
  });

  it('returns false on mismatch', async () => {
    const der = randomBytes(100);
    const h = createHash('sha256').update(der).digest();
    await expect(
      verifyTlsLeafBindsAttestation({ public_key: Uint8Array.from(h) }, randomBytes(100)),
    ).resolves.toBe(false);
  });

  it('returns false when public_key is missing or not 32 bytes', async () => {
    await expect(
      verifyTlsLeafBindsAttestation({ public_key: null }, randomBytes(10)),
    ).resolves.toBe(false);
    await expect(
      verifyTlsLeafBindsAttestation({ public_key: new Uint8Array(16) }, randomBytes(10)),
    ).resolves.toBe(false);
  });
});
