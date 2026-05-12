"use strict";

const express = require("express");
const axios = require("axios");

const PORT = parseInt(process.env.PORT || "8080", 10);
const app = express();

app.use(express.json());

app.get("/health", (_req, res) => {
  res.send("OK");
});

app.get("/egress", async (_req, res) => {
  try {
    const { data } = await axios.get("https://httpbin.org/ip");
    res.json(data);
  } catch (err) {
    console.error(`[server] egress error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.get("/attestation", async (req, res) => {
  try {
    const { data } = await axios.get("http://localhost:3000/attestation", { params: req.query });
    res.json(data);
  } catch (err) {
    console.error(`[server] attestation error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.post("/crypto", async (req, res) => {
  try {
    const body = req.body && req.body.plaintext != null ? req.body : { plaintext: "" };
    const { data: encrypted } = await axios.post("http://localhost:3000/encrypt", body, {
      headers: { "Content-Type": "application/json" },
    });
    const { data: decrypted } = await axios.post("http://localhost:3000/decrypt", { ciphertext: encrypted.data }, {
        headers: { "Content-Type": "application/json" },
    });
    res.json({ encrypted, decrypted });
  } catch (err) {
    console.error(`[server] crypto error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.post("/random", async (req, res) => {
  const { data } = await axios.post("http://localhost:3000/random", { length: req.body.length }, {
    headers: { "Content-Type": "application/json" },
  });
  res.json(data);
});

app.post("/kv", async (req, res) => {
  try {
    const { key = "hello/default", value = `kv-${Date.now()}` } = req.body || {};
    const headers = { "Content-Type": "application/json" };

    await axios.post("http://localhost:3000/kv/set", { key, value }, { headers });
    const { data } = await axios.post("http://localhost:3000/kv/get", { key }, { headers });

    res.json({ key, value: data.data });
  } catch (err) {
    console.error(`[server] kv error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.get("/env", async (_req, res) => {
  res.json({ env: process.env.DEMO });
});

app.listen(PORT, "0.0.0.0", () => {
  console.log(`[server] listening on port ${PORT}`);
});
