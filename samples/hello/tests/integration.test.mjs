/**
 * Integration tests for the hello enclave HTTP API.
 *
 * Set the enclave origin (no trailing slash) via:
 * - `ENCLAVE_URL` — primary (same as the sample client), or
 * - `NITRUM_E2E_BASE_URL` — fallback for the repo e2e scripts.
 *
 * Examples: deployed `https://…`; local `http://nitrum.local` or `http://localhost:8080`
 * after `nitrum local …`.
 *
 * Optional: `ENCLAVE_TLS_INSECURE=1` or `true` relaxes TLS verification for attestation
 * (debug / self-signed ingress only).
 */

import assert from "node:assert/strict";
import { before, describe, it } from "node:test";
import { NitrumVerifier } from "nitrum-node";

function resolveBaseUrl() {
  const raw = (process.env.ENCLAVE_URL || process.env.NITRUM_E2E_BASE_URL || "").trim();
  return raw.replace(/\/$/, "");
}

function isLocalDevelopment(url) {
  return url.includes("nitrum.local") || url.includes("localhost") || url.includes("127.0.0.1");
}

function tlsInsecureEnv() {
  return (
    process.env.ENCLAVE_TLS_INSECURE === "1" ||
    process.env.ENCLAVE_TLS_INSECURE === "true"
  );
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

describe("nitrum project (integration)", () => {
  before(() => {
    if (!baseUrl) {
      throw new Error(
        "Set ENCLAVE_URL or NITRUM_E2E_BASE_URL to the enclave origin (no trailing slash).",
      );
    }
  });

  describe("GET /.well-known/enclave/attestation", () => {
    if (isLocalDevelopment(baseUrl)) {
      it.skip("NitrumVerifier skipped in local development (non-production TLS)", () => {});
    } else {
      it("verifies attestation and TLS leaf binding", async () => {
        const verifier = new NitrumVerifier({
          baseUrl,
          tlsRejectUnauthorized: !tlsInsecureEnv(),
        });
        const doc = await verifier.verify();
        assert.ok(doc && typeof doc === "object");
      });
    }
  });

  it("GET /.well-known/enclave/status returns JSON", async () => {
    const data = await fetchJson(`${baseUrl}/.well-known/enclave/status`);
    assert.ok(data && typeof data === "object");
  });

  it("GET /health returns OK", async () => {
    const text = await fetchText(`${baseUrl}/health`);
    assert.equal(text.trim(), "OK");
  });

  it("GET /egress returns JSON (outbound reachability)", async () => {
    const data = await fetchJson(`${baseUrl}/egress`);
    assert.ok(data && typeof data === "object");
    assert.ok("origin" in data || "ip" in data);
  });

  it("POST /crypto round-trips plaintext", async () => {
    const data = await fetchJson(`${baseUrl}/crypto`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ plaintext: "hello" }),
    });
    assert.ok(data && typeof data === "object");
    assert.ok("encrypted" in data && "decrypted" in data);
    assert.equal(data.decrypted.plaintext, "hello");
  });

  it("POST /random returns JSON", async () => {
    const data = await fetchJson(`${baseUrl}/random`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    assert.ok(data && typeof data === "object");
  });

  it("GET /env returns JSON", async () => {
    const data = await fetchJson(`${baseUrl}/env`);
    assert.ok(data && typeof data === "object");
  });
});
