/**
 * @file index.ts
 * @description Verify AWS Nitro Enclave attestation documents.
 *
 * Node.js-focused verifier:
 * - {@link verifyAttestation} / {@link verifyTlsLeafBindsAttestation} use Node's `crypto`
 *   primitives and work in Node.js >= 18.
 * - {@link NitrumVerifier} assumes a Node.js environment and always performs TLS leaf binding.
 */

export type {
  AttestationDocument,
  AttestationFailure,
  AttestationResult,
  AttestationSuccess,
  ParsedAttestation,
  PCRMap,
  VerifyOptions,
} from "./types";

export { AWS_NITRO_ROOT_CA } from "./caroot";
export { verifyAttestation, verifyTlsLeafBindsAttestation } from "./attest-verify";
export { NitrumVerifier } from "./nitrum-verifier";
export type { NitrumVerifierOptions as NitrumVerifierInit } from "./nitrum-verifier";
