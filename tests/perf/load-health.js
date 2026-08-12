import http from "k6/http";
import { check } from "k6";

export const options = {
  vus: Number(__ENV.PERF_VUS || 50),
  duration: __ENV.PERF_DURATION || "30s",
  insecureSkipTLSVerify: true,
  thresholds: { http_req_failed: ["rate<0.01"] },
};

const BASE = __ENV.ENCLAVE_URL;

export default function () {
  const res = http.get(`${BASE}/health`);
  check(res, { "status is 200": (r) => r.status === 200 });
}
