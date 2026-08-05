# Macro load tests (EC2 Nitro)

Reproducible k6 load against a **deployed** Nitrum enclave. This harness does **not**
deploy or destroy CloudFormation stacks — leave that to `nitrum cloud deploy` /
`tests/e2e/cloud.sh`.

Local `nitrum local` + Docker is **not** a valid baseline (see
[`internal/PERFORMANCE.md`](../../internal/PERFORMANCE.md) Phase 5). Prefer a load
client on an EC2 instance in the **same VPC** as the stack; a laptop over the public
internet is fine for a smoke check but must be labeled as a WAN client in results.

## Prerequisites

- Deployed stack with a healthy enclave (e.g. `samples/hello` via `nitrum cloud deploy`)
- Enclave origin URL (`ENCLAVE_URL`, typically the NLB DNS from stack outputs)
- [`k6`](https://k6.io/) and [`jq`](https://jqlang.github.io/jq/) on the load client

```bash
brew install k6 jq   # macOS; on Linux use your distro / k6 install docs
```

## Quick start

```bash
# Optional: copy and edit env
cp tests/perf/.env.example tests/perf/.env

# From repo root:
ENCLAVE_URL=https://xxxx.elb.us-east-1.amazonaws.com ./tests/perf/run-macro.sh
```

Override load shape without a `.env` file:

```bash
ENCLAVE_URL=https://xxxx.elb.us-east-1.amazonaws.com \
  PERF_VUS=50 PERF_DURATION=30s \
  ./tests/perf/run-macro.sh
```

## What each script measures

| Script | Route | Meaning |
|--------|-------|---------|
| `load-health.js` | `GET /health` | TLS + ingress proxy + tiny app response (throughput ceiling) |
| `load-crypto.js` | `POST /crypto` | Proxy + in-enclave encrypt/decrypt via the hello sample (secure work path) |

Default `PERF_ROUTES=health,crypto`. Restrict with e.g. `PERF_ROUTES=crypto`.

## Output

Under `tests/perf/results/<UTC-timestamp>/` (gitignored):

- `health.json` / `crypto.json` — k6 `--summary-export`
- `summary.md` — markdown table (RPS, p50, p99, error rate) plus git SHA / target metadata

Paste the table into [`docs/performance_results.md`](../../docs/performance_results.md) after a real EC2 run.

## Procedure (story-grade numbers)

1. Deploy `samples/hello` (or equivalent) and wait until `GET /health` returns 200.
2. From a **same-VPC** EC2 client, install `k6` + `jq`, clone/checkout the same git SHA as the deployed EIF when possible.
3. Run `./tests/perf/run-macro.sh` with `ENCLAVE_URL` set (or document any `PERF_VUS` / `PERF_DURATION` overrides).
4. Confirm error rate ≈ 0% for both routes.
5. Copy the printed summary into `docs/performance_results.md` (macro section), noting instance type, enclave CPU/RAM, region, and client placement (same VPC vs WAN).
6. Destroy the stack when finished (`nitrum cloud destroy`) so you are not billed for an idle ASG.
