/**
 * Shared helpers for the Nitrum verifier.
 */

/** True when running under Node.js (used to gate dynamic `node:tls`, etc.). */
export function isNodeRuntime(): boolean {
  return (
    typeof process !== "undefined" &&
    process.versions != null &&
    typeof process.versions.node === "string"
  );
}

/** Fill bytes with CSPRNG from Web Crypto (`crypto.getRandomValues`). */
export function randomBytes(length: number): Uint8Array {
  const b = new Uint8Array(length);
  const c = globalThis.crypto;
  if (c?.getRandomValues) {
    c.getRandomValues(b);
    return b;
  }
  throw new Error("randomBytes: Web Crypto getRandomValues is not available");
}

/** Standard base64 encode (Node uses `Buffer`, browser uses `btoa`). */
export function bytesToBase64(bytes: Uint8Array): string {
  if (typeof Buffer !== "undefined") {
    return Buffer.from(bytes).toString("base64");
  }
  let bin = "";
  for (let i = 0; i < bytes.length; i++) {
    bin += String.fromCharCode(bytes[i]);
  }
  return btoa(bin);
}

/** Standard base64 decode to a new `Uint8Array`. */
export function base64ToBytes(b64: string): Uint8Array {
  if (typeof Buffer !== "undefined") {
    return Uint8Array.from(Buffer.from(b64, "base64"));
  }
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) {
    out[i] = bin.charCodeAt(i);
  }
  return out;
}

/**
 * Convert a `Uint8Array` or Node `Buffer` view into a tightly packed `ArrayBuffer`.
 */
export function toArrayBuffer(u: Uint8Array | Buffer): ArrayBuffer {
  if (
    u.buffer instanceof ArrayBuffer &&
    u.byteOffset === 0 &&
    u.byteLength === u.buffer.byteLength
  ) {
    return u.buffer as ArrayBuffer;
  }
  return u.buffer.slice(u.byteOffset, u.byteOffset + u.byteLength) as ArrayBuffer;
}
