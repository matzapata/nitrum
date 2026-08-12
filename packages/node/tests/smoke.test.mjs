/**
 * Smoke tests: confirm the napi binding loads and forwards to `crates/verify`.
 * Crypto / policy coverage lives in `crates/verify` Rust tests.
 */

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { verifyAttestation, verifyTlsLeafBindsAttestation } = require("../index.js");

const fixtures = join(
  dirname(fileURLToPath(import.meta.url)),
  "../../../crates/verify/tests/fixtures",
);

function load(name) {
  const text = readFileSync(join(fixtures, name), "utf8").trim();
  return Uint8Array.from(Buffer.from(text, "base64"));
}

describe("nitrum-node binding", () => {
  it("accepts valid.cbor", () => {
    const result = verifyAttestation(load("valid.cbor"));
    assert.equal(result.valid, true);
    assert.ok(result.document?.module_id);
  });

  it("rejects invalid-signature.cbor", () => {
    const result = verifyAttestation(load("invalid-signature.cbor"));
    assert.equal(result.valid, false);
    assert.ok(result.reason);
  });

  it("forwards verifyTlsLeafBindsAttestation", () => {
    const der = Uint8Array.from({ length: 100 }, () => 1);
    const digest = createHash("sha256").update(der).digest();
    assert.equal(verifyTlsLeafBindsAttestation(digest, der), true);
    assert.equal(verifyTlsLeafBindsAttestation(digest, Uint8Array.from({ length: 100 }, () => 2)), false);
    assert.equal(verifyTlsLeafBindsAttestation(null, der), false);
  });
});
