#!/usr/bin/env bash
# Reproducible macro load test against a deployed Nitrum enclave (EC2 Nitro).
#
# Assumes the CloudFormation stack is already up (nitrum cloud deploy / e2e cloud.sh).
# Does not deploy or destroy resources.
#
# Usage (from repo root):
#   ./tests/perf/run-macro.sh
#
# See tests/perf/README.md and tests/perf/.env.example.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

if [[ -f "${SCRIPT_DIR}/.env" ]]; then
    set -a
    # shellcheck source=/dev/null
    source "${SCRIPT_DIR}/.env"
    set +a
fi

ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"
PERF_VUS="${PERF_VUS:-50}"
PERF_DURATION="${PERF_DURATION:-30s}"
PERF_ROUTES="${PERF_ROUTES:-health,crypto}"
WAIT_ACTIVE_TIMEOUT_SECONDS="${WAIT_ACTIVE_TIMEOUT_SECONDS:-300}"
WAIT_ACTIVE_POLL_SECONDS="${WAIT_ACTIVE_POLL_SECONDS:-5}"

TIMESTAMP="$(date -u +%Y%m%dT%H%M%SZ)"
RESULTS_DIR="${PERF_RESULTS_DIR:-${SCRIPT_DIR}/results/${TIMESTAMP}}"

trim() {
    # shellcheck disable=SC2001
    echo "$1" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//'
}

script_for_route() {
    case "$1" in
        health) echo "load-health.js" ;;
        crypto) echo "load-crypto.js" ;;
        *) return 1 ;;
    esac
}

label_for_route() {
    case "$1" in
        health) echo "GET /health" ;;
        crypto) echo "POST /crypto" ;;
        *) echo "$1" ;;
    esac
}

require_tools() {
    local missing=0
    if ! command -v k6 >/dev/null 2>&1; then
        echo "error: k6 not found (install: brew install k6)" >&2
        missing=1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        echo "error: jq not found (install: brew install jq)" >&2
        missing=1
    fi
    if ! command -v curl >/dev/null 2>&1; then
        echo "error: curl not found" >&2
        missing=1
    fi
    if [[ -z "${ENCLAVE_URL:-}" ]]; then
        echo "error: set ENCLAVE_URL (e.g. https://xxxx.elb.us-east-1.amazonaws.com)" >&2
        missing=1
    fi
    if ((missing)); then
        exit 1
    fi
}

wait_active() {
    local base="$1"
    echo "=== wait active (timeout ${WAIT_ACTIVE_TIMEOUT_SECONDS}s): ${base}/health ==="

    local deadline now remaining sleep_for
    deadline=$((SECONDS + WAIT_ACTIVE_TIMEOUT_SECONDS))

    local -a curl_args
    curl_args=(-sS -o /dev/null --connect-timeout 5 --max-time 10)
    if [[ "${ENCLAVE_TLS_INSECURE}" == "1" ]]; then
        curl_args+=(-k)
    fi

    while true; do
        if curl "${curl_args[@]}" "${base}/health"; then
            echo "=== enclave is responsive ==="
            return 0
        fi

        now=${SECONDS}
        if ((now >= deadline)); then
            echo "error: enclave did not become responsive within ${WAIT_ACTIVE_TIMEOUT_SECONDS}s (${base}/health)" >&2
            exit 1
        fi

        remaining=$((deadline - now))
        echo "waiting for enclave to respond... (${remaining}s remaining)"
        sleep_for="${WAIT_ACTIVE_POLL_SECONDS}"
        if ((remaining < WAIT_ACTIVE_POLL_SECONDS)); then
            sleep_for="${remaining}"
        fi
        sleep "${sleep_for}"
    done
}

run_scenario() {
    local name="$1"
    local script="$2"
    local out_json="${RESULTS_DIR}/${name}.json"

    echo "=== k6 ${name} (vus=${PERF_VUS} duration=${PERF_DURATION}) ==="
    # Include p(99) in the export; default trend stats stop at p(95).
    # --summary-export uses a flat metric shape (no .values nesting).
    k6 run \
        --insecure-skip-tls-verify \
        --summary-trend-stats="med,avg,p(90),p(95),p(99)" \
        -e "ENCLAVE_URL=${ENCLAVE_URL}" \
        -e "PERF_VUS=${PERF_VUS}" \
        -e "PERF_DURATION=${PERF_DURATION}" \
        --summary-export="${out_json}" \
        "${SCRIPT_DIR}/${script}"
}

metric_or_na() {
    local file="$1"
    local path="$2"
    if [[ ! -f "${file}" ]]; then
        echo "n/a"
        return 0
    fi
    local value
    value="$(jq -r "${path} // empty" "${file}" 2>/dev/null || true)"
    if [[ -z "${value}" || "${value}" == "null" ]]; then
        echo "n/a"
    else
        echo "${value}"
    fi
}

format_pct() {
    local rate="$1"
    if [[ "${rate}" == "n/a" ]]; then
        echo "n/a"
        return 0
    fi
    awk -v r="${rate}" 'BEGIN { printf "%.1f%%", r * 100 }'
}

format_ms() {
    local ms="$1"
    if [[ "${ms}" == "n/a" ]]; then
        echo "n/a"
        return 0
    fi
    awk -v m="${ms}" 'BEGIN { printf "%.2f", m }'
}

format_rps() {
    local rps="$1"
    if [[ "${rps}" == "n/a" ]]; then
        echo "n/a"
        return 0
    fi
    awk -v r="${rps}" 'BEGIN { printf "%.1f", r }'
}

print_summary() {
    local git_sha
    git_sha="$(git -C "${REPO_ROOT}" rev-parse --short HEAD 2>/dev/null || echo unknown)"

    local summary_file="${RESULTS_DIR}/summary.md"
    {
        echo "# Macro load summary"
        echo
        echo "- Generated (UTC): ${TIMESTAMP}"
        echo "- Git SHA: \`${git_sha}\`"
        echo "- Target: \`${ENCLAVE_URL}\`"
        echo "- Concurrency (VUs): ${PERF_VUS}"
        echo "- Duration: ${PERF_DURATION}"
        echo "- Routes: ${PERF_ROUTES}"
        echo
        echo "| Route | Concurrency | Duration | RPS | p50 (ms) | p99 (ms) | Error rate |"
        echo "|-------|-------------|----------|-----|----------|----------|------------|"
    } >"${summary_file}"

    local route script json rps p50 p99 err_rate err_pct route_label
    local IFS=','
    # shellcheck disable=SC2086
    set -- ${PERF_ROUTES}
    for route in "$@"; do
        route="$(trim "${route}")"
        [[ -n "${route}" ]] || continue
        if ! script="$(script_for_route "${route}")"; then
            echo "warning: unknown route '${route}' (skipping summary row)" >&2
            continue
        fi
        json="${RESULTS_DIR}/${route}.json"
        # k6 --summary-export layout: .metrics.<name>.{rate,med,p(99),value} (flat).
        rps="$(format_rps "$(metric_or_na "${json}" '.metrics.http_reqs.rate')")"
        p50="$(format_ms "$(metric_or_na "${json}" '.metrics.http_req_duration.med')")"
        p99="$(format_ms "$(metric_or_na "${json}" '.metrics.http_req_duration["p(99)"]')")"
        err_rate="$(metric_or_na "${json}" '.metrics.http_req_failed.value')"
        err_pct="$(format_pct "${err_rate}")"
        route_label="$(label_for_route "${route}")"
        echo "| ${route_label} | ${PERF_VUS} | ${PERF_DURATION} | ${rps} | ${p50} | ${p99} | ${err_pct} |" >>"${summary_file}"
    done

    echo
    echo "=== summary (${summary_file}) ==="
    cat "${summary_file}"
}

main() {
    require_tools
    mkdir -p "${RESULTS_DIR}"

    ENCLAVE_URL="${ENCLAVE_URL%/}"
    export ENCLAVE_URL
    echo "=== target ${ENCLAVE_URL} ==="

    wait_active "${ENCLAVE_URL}"

    local route script
    local ran=0
    local IFS=','
    # shellcheck disable=SC2086
    set -- ${PERF_ROUTES}
    for route in "$@"; do
        route="$(trim "${route}")"
        [[ -n "${route}" ]] || continue
        if ! script="$(script_for_route "${route}")"; then
            echo "error: unknown route '${route}' (allowed: health,crypto)" >&2
            exit 1
        fi
        run_scenario "${route}" "${script}"
        ran=1
    done
    if ((ran == 0)); then
        echo "error: PERF_ROUTES is empty" >&2
        exit 1
    fi

    print_summary
    echo "=== results directory: ${RESULTS_DIR} ==="
}

main "$@"
