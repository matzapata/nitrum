"use strict";

const express = require("express");
const axios = require("axios");

const PORT = parseInt(process.env.PORT || "8008", 10);
const app = express();

app.get("/health", (req, res) => {
  res.send("OK");
});

app.get("/egress", async (req, res) => {
  try {
    const { data } = await axios.get("http://httpbin.org/ip");
    res.json(data);
  } catch (err) {
    console.error(`[server] egress error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.post("/attestation", async (req, res) => {
  try {
    const { data } = await axios.post("http://localhost:3000/attestation", req.body);
    res.json(data);
  } catch (err) {
    console.error(`[server] attestation error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.post("/crypto", async (req, res) => {
  try {
  const { data: encrypted } = await axios.post("http://localhost:3000/encrypt", req.body);
    const { data: decrypted } = await axios.post("http://localhost:3000/decrypt", encrypted);
    res.json({ encrypted, decrypted });
  } catch (err) {
    console.error(`[server] crypto error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.listen(PORT, "0.0.0.0", () => {
  console.log(`[server] listening on port ${PORT}`);
});
