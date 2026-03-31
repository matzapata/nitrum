import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { NitrumVerifier } from "nitrum-node";

const baseUrl = process.env.ENCLAVE_URL as string;
if (!baseUrl) {
  throw new Error("ENCLAVE_URL is not set");
}

async function runStep<T>(label: string, fn: () => Promise<T>): Promise<void> {
  console.log(`\n--- ${label} ---`);
  const out = await fn();
  if (out !== undefined) {
    console.log(typeof out === "string" ? out : JSON.stringify(out, null, 2));
  }
}

export async function main(): Promise<void> {
  console.log("ENCLAVE_URL:", baseUrl);

  await runStep("GET /.well-known/enclave/attestation with verification", async () => {
    const verifier = new NitrumVerifier({
      baseUrl,
      tlsRejectUnauthorized: process.env.ENCLAVE_TLS_INSECURE === "1" || process.env.ENCLAVE_TLS_INSECURE === "true",
    });

    return await verifier.verify();
  });

  await runStep("GET /.well-known/enclave/status", async () => {
    const res = await fetch(`${baseUrl}/.well-known/enclave/status`);
    const text = await res.text();
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${text}`);
    return JSON.parse(text) as unknown;
  });

  await runStep("GET /health", async () => {
    const res = await fetch(`${baseUrl}/health`);
    const text = await res.text();
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${text}`);
    return text;
  });

  await runStep("GET /egress", async () => {
    const res = await fetch(`${baseUrl}/egress`);
    const text = await res.text();
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${text}`);
    return JSON.parse(text) as unknown;
  });

  await runStep("POST /crypto", async () => {
    const res = await fetch(`${baseUrl}/crypto`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ plaintext: "hello" }),
    });
    const text = await res.text();
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${text}`);
    return JSON.parse(text) as unknown;
  });

  await runStep("POST /random", async () => {
    const res = await fetch(`${baseUrl}/random`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({}),
    });
    const text = await res.text();
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${text}`);
    return JSON.parse(text) as unknown;
  });

  await runStep("GET /env", async () => {
    const res = await fetch(`${baseUrl}/env`);
    const text = await res.text();
    if (!res.ok) throw new Error(`HTTP ${res.status}: ${text}`);
    return JSON.parse(text) as unknown;
  });
}

if (resolve(fileURLToPath(import.meta.url)) === resolve(process.argv[1] ?? "")) {
  main().catch((err: unknown) => {
    console.error(err);
    process.exitCode = 1;
  });
}
