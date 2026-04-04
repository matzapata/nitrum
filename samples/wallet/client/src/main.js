import crypto from "crypto";
import axios from "axios";
import https from "https";

const baseUrl = "https://nitrum.local";

// For local development with a self-signed certificate.
// Do NOT use this in production.
const httpsAgent = new https.Agent({ rejectUnauthorized: false });

async function runDemo() {
  console.log("=== Blockchain Wallet Toy Demo ===");
  console.log("ENCLAVE_URL:", baseUrl);

  const clientSecret = "my-very-private-secret";

  // Attestation verification
  const verifier = new NitrumVerifier({
    baseUrl,
    tlsRejectUnauthorized:
      process.env.ENCLAVE_TLS_INSECURE === "1" ||
      process.env.ENCLAVE_TLS_INSECURE === "true",
  });
  await verifier.verify();

  // 1) Ask the enclave to generate and encrypt a new wallet key
  const generateRes = await axios.post(
    `${baseUrl}/wallet`,
    { secret: clientSecret },
    { headers: { "Content-Type": "application/json" }, httpsAgent },
  );
  const { ciphertext } = generateRes.data;
  console.log("[client] Received ciphertext from enclave:", ciphertext);

  // 2) Prepare a fake Ethereum transaction to sign
  const txData = {
    to: "0x1234abcd00000000000000000000000000000000",
    nonce: 1n,
    gasLimit: 21_000n,
    maxFeePerGas: 30_000_000_000n,
    maxPriorityFeePerGas: 2_000_000_000n,
    chainId: 1n,
    // Represent value in wei so the enclave can treat it as BigInt directly
    value: 500_000_000_000_000_000n, // 0.5 ETH
  };

  const proof = crypto
    .createHmac("sha256", Buffer.from(clientSecret, "utf8"))
    .update(
      JSON.stringify(
        txData,
        (_key, value) => (typeof value === "bigint" ? value.toString() : value),
      ),
    )
    .digest("hex");

  console.log("[client] Computed HMAC over txData:", proof);

  // 3) Ask enclave to sign the transaction
  const signRes = await axios.post(
    `${baseUrl}/wallet/sign`,
    {
      ciphertext,
      txData: JSON.parse(
        JSON.stringify(
          txData,
          (_key, value) => (typeof value === "bigint" ? value.toString() : value),
        ),
      ),
      proof,
    },
    { headers: { "Content-Type": "application/json" }, httpsAgent },
  );

  const { signature } = signRes.data;
  console.log("[client] Received signature:", signature);
  console.log("=== Demo complete ===");
}

runDemo().catch((err) => {
  console.error(err);
  process.exit(1);
});
