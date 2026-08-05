import http from "k6/http";
import { check } from "k6";

export const options = {
  vus: Number(__ENV.PERF_VUS || 50),
  duration: __ENV.PERF_DURATION || "30s",
  insecureSkipTLSVerify: true,
  thresholds: { http_req_failed: ["rate<0.01"] },
};

const BASE = __ENV.ENCLAVE_URL;
const PAYLOAD = JSON.stringify({ plaintext: "nitrum-macro-load-test" });
const PARAMS = { headers: { "Content-Type": "application/json" } };

export default function () {
  const res = http.post(`${BASE}/crypto`, PAYLOAD, PARAMS);
  check(res, { "status is 200": (r) => r.status === 200 });
}
