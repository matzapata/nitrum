# nitrum-node

Verify [AWS Nitro Enclave](https://docs.aws.amazon.com/enclaves/latest/user/nitro-enclaves-concepts.html) attestation documents in Node.js.

Thin napi-rs bindings over `crates/verify` (COSE, certificate chain, PCRs, nonce, TLS leaf hash bind). High-level fetch / orchestration is left to the caller.

## Installation

```bash
npm install nitrum-node
```

**Native addon:** this package compiles Rust via `napi` on `npm install` (`prepare`). You need a **Rust toolchain** (rustc/cargo, MSRV 1.95+) unless you consume a release that ships prebuilt `*.node` binaries for your platform (not published yet — from-source install only for now).

## Usage

```js
const {
  verifyAttestation,
  verifyTlsLeafBindsAttestation,
  awsNitroRootCa,
} = require("nitrum-node");

const result = verifyAttestation(rawDocument, {
  expectedPcrs: { pcr0: EXPECTED_PCR0 },
  nonce,
  maxAgeMs: 5 * 60_000,
});

if (!result.valid) {
  throw new Error(result.reason);
}

const binds = verifyTlsLeafBindsAttestation(result.document.public_key, tlsLeafDer);
if (!binds) {
  throw new Error("TLS leaf does not match attestation public_key");
}
```

`awsNitroRootCa()` returns the AWS Nitro root CA PEM used by default.

## Development

```bash
npm install
npm run build -w nitrum-node
npm test -w nitrum-node
```

Requires a Rust toolchain (native addon build). Full verification coverage lives in `crates/verify`.
