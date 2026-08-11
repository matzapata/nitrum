/**
 * Integration tests for the wallet enclave HTTP API.
 *
 * Set the enclave origin (no trailing slash) via:
 * - `ENCLAVE_URL` — primary, or
 * - `NITRUM_E2E_BASE_URL` — fallback for repo e2e scripts.
 *
 * Optional: `ENCLAVE_TLS_INSECURE=1` or `true` relaxes TLS verification for
 * attestation (debug / self-signed ingress only).
 */

import assert from "node:assert/strict";
import crypto, { randomBytes } from "node:crypto";
import { createRequire } from "node:module";
import tls from "node:tls";
import { before, describe, it } from "node:test";

const require = createRequire(import.meta.url);
const { verifyAttestation, verifyTlsLeafBindsAttestation } = require("nitrum-node");

function resolveBaseUrl() {
  const raw = (
    process.env.ENCLAVE_URL ||
    process.env.NITRUM_E2E_BASE_URL ||
    "https://127.0.0.1:443"
  ).trim();
  return raw.replace(/\/$/, "");
}

function isLocalDevelopment(url) {
  return (
    url.includes("nitrum.localhost") ||
    url.includes("localhost") ||
    url.includes("127.0.0.1")
  );
}

function tlsInsecureEnv() {
  return (
    process.env.ENCLAVE_TLS_INSECURE === "1" ||
    process.env.ENCLAVE_TLS_INSECURE === "true"
  );
}

/** Fetch attestation, verify document + TLS leaf binding. */
async function verifyEnclaveAttestation(origin, { rejectUnauthorized }) {
  const nonce = randomBytes(32);
  const url = `${origin}/.well-known/enclave/attestation?nonce=${encodeURIComponent(nonce.toString("base64"))}`;
  const res = await fetch(url);
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`attestation HTTP ${res.status}: ${text.slice(0, 500)}`);
  }
  const body = JSON.parse(text);
  if (body.error) throw new Error(body.error);
  if (!body.data) throw new Error("attestation: missing document");

  const raw = Uint8Array.from(Buffer.from(body.data, "base64"));
  const result = await verifyAttestation(raw, {
    nonce: Uint8Array.from(nonce),
    maxAgeMs: 5 * 60_000,
  });
  if (!result.valid) {
    throw new Error(result.reason ?? "verifyAttestation failed");
  }

  const u = new URL(origin.startsWith("http") ? origin : `https://${origin}`);
  if (u.protocol !== "https:") {
    throw new Error("attestation TLS binding requires https:");
  }
  const port = u.port ? Number(u.port) : 443;
  const leafDer = await getTlsLeafDer(u.hostname, port, rejectUnauthorized);
  if (!(await verifyTlsLeafBindsAttestation(result.document, leafDer))) {
    throw new Error("TLS leaf does not match attestation public_key");
  }
  return result.document;
}

function getTlsLeafDer(hostname, port, rejectUnauthorized) {
  return new Promise((resolve, reject) => {
    const socket = tls.connect(
      { host: hostname, port, servername: hostname, rejectUnauthorized },
      () => {
        try {
          const cert = socket.getPeerCertificate();
          socket.end();
          if (!cert?.raw || typeof cert.raw === "string") {
            reject(new Error("TLS: no peer certificate DER"));
            return;
          }
          resolve(new Uint8Array(cert.raw));
        } catch (e) {
          socket.end();
          reject(e);
        }
      },
    );
    socket.on("error", reject);
  });
}

const baseUrl = resolveBaseUrl();

if (tlsInsecureEnv()) {
  process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";
}

async function fetchText(url) {
  const res = await fetch(url);
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}: ${text}`);
  }
  return text;
}

async function fetchJson(url, init) {
  const res = await fetch(url, init);
  const text = await res.text();
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}: ${text}`);
  }
  return JSON.parse(text);
}

describe("blockchain-wallet (integration)", () => {
  before(() => {
    if (!baseUrl) {
      throw new Error(
        "Set ENCLAVE_URL or NITRUM_E2E_BASE_URL to the enclave origin (no trailing slash).",
      );
    }
  });

  if (isLocalDevelopment(baseUrl)) {
    it.skip(
      "GET /.well-known/enclave/attestation skipped in local development (non-production TLS)",
      () => {},
    );
  } else {
    it("GET /.well-known/enclave/attestation validates attestation and TLS leaf binding", async () => {
      const doc = await verifyEnclaveAttestation(baseUrl, {
        rejectUnauthorized: !tlsInsecureEnv(),
      });
      assert.ok(doc && typeof doc === "object");
    });
  }

  it("GET /.well-known/enclave/status returns JSON", async () => {
    const data = await fetchJson(`${baseUrl}/.well-known/enclave/status`);
    assert.ok(data && typeof data === "object");
    assert.equal(data.status, "ok");
  });

  it("GET /health returns OK", async () => {
    const text = await fetchText(`${baseUrl}/health`);
    assert.equal(text.trim(), "OK");
  });

  it("POST /wallet then /wallet/sign creates and signs a transaction", async () => {
    const clientSecret = "my-very-private-secret";

    const { ciphertext } = await fetchJson(`${baseUrl}/wallet`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ secret: clientSecret }),
    });
    assert.ok(typeof ciphertext === "string" && ciphertext.length > 0);

    const txData = {
      to: "0x1234abcd00000000000000000000000000000000",
      nonce: 1n,
      gasLimit: 21_000n,
      maxFeePerGas: 30_000_000_000n,
      maxPriorityFeePerGas: 2_000_000_000n,
      chainId: 1n,
      value: 500_000_000_000_000_000n,
    };

    const jsonReplacer = (_key, value) =>
      typeof value === "bigint" ? value.toString() : value;

    const proof = crypto
      .createHmac("sha256", Buffer.from(clientSecret, "utf8"))
      .update(JSON.stringify(txData, jsonReplacer))
      .digest("hex");

    const { signature } = await fetchJson(`${baseUrl}/wallet/sign`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(
        {
          ciphertext,
          txData: JSON.parse(JSON.stringify(txData, jsonReplacer)),
          proof,
        },
      ),
    });

    assert.ok(typeof signature === "string" && signature.startsWith("0x"));
  });
});
