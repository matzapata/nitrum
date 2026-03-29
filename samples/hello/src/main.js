"use strict";

const express = require("express");
const axios = require("axios");

const PORT = parseInt(process.env.PORT || "8080", 10);
const app = express();

app.use(express.json());

app.get("/health", (req, res) => {
  res.send("OK");
});

app.get("/egress", async (req, res) => {
  try {
    const { data } = await axios.get("https://httpbin.org/ip");
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
    const body = req.body && req.body.plaintext != null ? req.body : { plaintext: "" };
    const { data: encrypted } = await axios.post("http://localhost:3000/encrypt", body, {
      headers: { "Content-Type": "application/json" },
    });
    const { data: decrypted } = await axios.post("http://localhost:3000/decrypt", encrypted, {
      headers: { "Content-Type": "application/json" },
    });
    res.json({ encrypted, decrypted });
  } catch (err) {
    console.error(`[server] crypto error: ${err.message}`);
    res.status(502).json({ error: err.message });
  }
});

app.post("/random", async (req, res) => {
  const { data } = await axios.post("http://localhost:3000/random", req.body, {
    headers: { "Content-Type": "application/json" },
  });
  res.json(data);
});

app.get("/env", async (req, res) => {
  res.json({ env: process.env.DEMO });
});

app.listen(PORT, "0.0.0.0", () => {
  console.log(`[server] listening on port ${PORT}`);
});
