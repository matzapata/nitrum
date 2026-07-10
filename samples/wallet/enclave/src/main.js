"use strict";

require("./instrumentation");

const express = require("express");
const axios = require("axios");
const crypto = require("crypto");
const { Wallet } = require("ethers");
const { metrics } = require("@opentelemetry/api");

const meter = metrics.getMeter("blockchain-wallet");
const walletsCreated = meter.createCounter("app.wallet.created", {
  description: "Wallets created by the sample app",
});
const signLatency = meter.createHistogram("app.wallet.sign.duration.ms", {
  description: "Wallet sign request latency in the sample app",
  unit: "ms",
});
const signErrors = meter.createCounter("app.wallet.sign.errors", {
  description: "Failed wallet sign requests in the sample app",
});

const PORT = parseInt(process.env.PORT || "8080", 10);
const app = express();

// ---------------------------------------------------------------------------
// Nitrum utilities
// ---------------------------------------------------------------------------

async function encrypt(plaintext) {
  const { data: { data, error } } = await axios.post(
    "http://localhost:3000/encrypt",
    { plaintext },
    { headers: { "Content-Type": "application/json" } },
  );
  if (error) {
    throw new Error(`[enclave] encrypt error: ${error}`);
  }
  return data;
}

async function decrypt(ciphertext) {
  const res = await axios.post(
    "http://localhost:3000/decrypt",
    { ciphertext },
    { headers: { "Content-Type": "application/json" } },
  );
  if (res.data.error) {
    throw new Error(`[enclave] decrypt error: ${res.data.error}`);
  }
  return res.data.data;
}

async function random(length) {
  const { data: { data, error } } = await axios.post(
    "http://localhost:3000/random",
    { length },
    { headers: { "Content-Type": "application/json" } },
  );
  if (error) {
    throw new Error(`[enclave] random error: ${error}`);
  }
  return Buffer.from(data, "base64");
}

async function kvSet(key, value) {
  const { data: { data, error } } = await axios.post(
    "http://localhost:3000/kv/set",
    { key, value },
    { headers: { "Content-Type": "application/json" } },
  );
  if (error) {
    throw new Error(`[enclave] kv set error: ${error}`);
  }
  return data;
}

// ---------------------------------------------------------------------------
// HTTP surface for the enclave
// ---------------------------------------------------------------------------

// Parse JSON bodies
app.use(express.json());

// GET /health
app.get("/health", (_req, res) => {
  res.send("OK");
});

// POST /wallet { secret }
app.post("/wallet", async (req, res) => {
  try {
    // Validate the request body
    const { secret } = req.body || {};
    if (typeof secret !== "string" || !secret.length) {
      return res.status(400).json({ error: "secret is required" });
    }

    // Leverage data-plane random endpoint to derive a fresh 32-byte private key
    const privateKey = await random(32);

    // Encrypt the private key and secret using the data-plane encrypt endpoint
    const ciphertext = await encrypt(JSON.stringify({
      privateKey: privateKey.toString("hex"),
      secret,
    }));

    await kvSet("wallet:demo_last_ciphertext", ciphertext);

    walletsCreated.add(1);
    res.json({ ciphertext });
  } catch (err) {
    res.status(500).json({ error: "failed to generate key" });
  }
});

// POST /wallet/sign { ciphertext, txData, proof }
app.post("/wallet/sign", async (req, res) => {
  const start = performance.now();
  try {
    const { ciphertext, txData, proof } = req.body || {};
    if (!ciphertext || !txData || typeof proof !== "string") {
      return res
        .status(400)
        .json({ error: "encrypted, txData, and proof are required" });
    }

    // Decrypt the private key and secret using the data-plane decrypt endpoint
    const decrypted = await decrypt(ciphertext);
    const { secret, privateKey } = JSON.parse(decrypted);

    // Verify the proof using the secret
    const expectedProof = crypto
      .createHmac("sha256", Buffer.from(secret, "utf8"))
      .update(
        JSON.stringify(txData, (_key, value) =>
          typeof value === "bigint" ? value.toString() : value,
        ),
      )
      .digest("hex");
    if (expectedProof !== proof) {
      throw new Error(
        "HMAC verification failed - tampered data or wrong secret",
      );
    }

    // Sign the transaction using the private key
    const tx = {
      to: txData.to,
      nonce: BigInt(txData.nonce),
      chainId: BigInt(txData.chainId),
      gasLimit: BigInt(txData.gasLimit),
      maxFeePerGas: BigInt(txData.maxFeePerGas),
      maxPriorityFeePerGas: BigInt(txData.maxPriorityFeePerGas),
      value: BigInt(txData.value),
      type: 2,
    };
    const signature = await new Wallet("0x" + privateKey).signTransaction(tx);

    // Return the signed transaction
    res.json({ signature });
  } catch (err) {
    signErrors.add(1, { reason: "failed" });
    res.status(500).json({ error: "failed to sign transaction" });
  } finally {
    signLatency.record(performance.now() - start, { route: "/wallet/sign" });
  }
});

app.listen(PORT, "0.0.0.0", () => {
  console.log(`[enclave] blockchain-wallet listening on port ${PORT}`);
});
