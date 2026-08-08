#!/usr/bin/env bash
# Linear workflow: init → local up → wait ready → local logs → npm integration tests → local down.
#
# Environment (see tests/e2e/.env.example):
#   NITRUM_BIN               optional override command; default:
#                            cargo run -q ... (quiet compile; see NITRUM_E2E_RUST_LOG)
#   NITRUM_E2E_RUST_LOG      tracing filter for default cargo path (default: warn,cli=info)
#   NITRUM_E2E_PARENT_DIR, NITRUM_E2E_INIT_NAME, PROJECT path
#   ENCLAVE_URL              default: https://127.0.0.1:443 (override for https://nitrum.local if preferred)
#   ENCLAVE_TLS_INSECURE     default: 1
#   NITRUM_LOCAL_LOGS_TAIL   passed to `nitrum local logs --tail` (default: 80)
#   NITRUM_LOCAL_WAIT_READY_TIMEOUT_SECONDS  default 60
#   NITRUM_LOCAL_WAIT_READY_POLL_SECONDS     default 1
#
# Runtime images come from the project's nitrum.toml `[runtime]` section, optionally
# overridden via NITRUM_RUNTIME_DATA_PLANE_IMAGE (and siblings). `nitrum local up`
# automatically appends `-local` to the resolved data_plane tag. Build images with
# `docker buildx bake` — see CONTRIBUTING.md. This script does not build images.
#
# Add 127.0.0.1 nitrum.local to /etc/hosts if needed.
#
# Usage: from repo root, `./tests/e2e/local.sh`
# The generated workspace under `${NITRUM_E2E_PARENT_DIR}/${NITRUM_E2E_INIT_NAME}` is recreated on each run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib/pin-sdk.sh"

if [[ -f "${SCRIPT_DIR}/.env" ]]; then
    set -a
    # shellcheck source=/dev/null
    source "${SCRIPT_DIR}/.env"
    set +a
fi

PARENT="${NITRUM_E2E_PARENT_DIR:-${REPO_ROOT}/target/nitrum-e2e-workspace}"
NAME="${NITRUM_E2E_INIT_NAME:-nitrum-e2e-demo}"
PROJECT="${PARENT}/${NAME}"
LOG_TAIL="${NITRUM_LOCAL_LOGS_TAIL:-80}"
WAIT_READY_TIMEOUT_SECONDS="${NITRUM_LOCAL_WAIT_READY_TIMEOUT_SECONDS:-60}"
WAIT_READY_POLL_SECONDS="${NITRUM_LOCAL_WAIT_READY_POLL_SECONDS:-1}"
STACK_UP=0

nitrum() {
    local -a cmd
    if [[ -n "${NITRUM_BIN:-}" ]]; then
        # shellcheck disable=SC2206
        cmd=(${NITRUM_BIN})
        "${cmd[@]}" -- "$@"
    else
        RUST_LOG="${NITRUM_E2E_RUST_LOG:-warn,cli=info}" \
            cargo run -q --manifest-path "${REPO_ROOT}/Cargo.toml" --bin nitrum -- "$@"
    fi
}

step_init() {
    mkdir -p "${PARENT}"
    if [[ -e "${PROJECT}" ]]; then
        echo "=== init: reset ${PROJECT} ==="
        rm -rf "${PROJECT}"
    fi
    echo "=== init: ${NAME} in ${PARENT} ==="
    (cd "${PARENT}" && nitrum init "${NAME}")
}

cleanup() {
    if [[ "${STACK_UP}" == "1" && -f "${PROJECT}/nitrum.toml" ]]; then
        echo "=== cleanup: local down ==="
        (cd "${PROJECT}" && nitrum local down) || true
    fi
}

preclean_stack() {
    if [[ -f "${PROJECT}/nitrum.toml" ]]; then
        echo "=== pre-clean: local down ==="
        (cd "${PROJECT}" && nitrum local down) || true
    fi
}

step_up() {
    echo "=== local up ==="
    (cd "${PROJECT}" && nitrum local up)
    STACK_UP=1
}

# Wait until /.well-known/enclave/status returns HTTP 200 (app health probe ready).
step_wait_ready() {
    command -v curl >/dev/null 2>&1 || {
        echo "error: curl not found (needed to wait for enclave readiness)" >&2
        exit 1
    }

    export ENCLAVE_URL="${ENCLAVE_URL:-https://127.0.0.1:443}"
    export ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"
    local status_url="${ENCLAVE_URL%/}/.well-known/enclave/status"

    echo "=== wait ready (timeout ${WAIT_READY_TIMEOUT_SECONDS}s): ${status_url} ==="

    local deadline now remaining sleep_for http_code
    deadline=$((SECONDS + WAIT_READY_TIMEOUT_SECONDS))

    local -a curl_args
    curl_args=(-sS -o /dev/null -w '%{http_code}' --connect-timeout 2 --max-time 5)
    if [[ "${ENCLAVE_TLS_INSECURE}" == "1" ]]; then
        curl_args+=(-k)
    fi

    while true; do
        http_code="$(curl "${curl_args[@]}" "${status_url}" || true)"
        if [[ "${http_code}" == "200" ]]; then
            echo "=== enclave status ready ==="
            return 0
        fi

        now=${SECONDS}
        if ((now >= deadline)); then
            echo "error: enclave status not ready within ${WAIT_READY_TIMEOUT_SECONDS}s (last HTTP ${http_code:-none}; ${status_url})" >&2
            exit 1
        fi

        remaining=$((deadline - now))
        echo "waiting for enclave status... (HTTP ${http_code:-none}, ${remaining}s remaining)"
        sleep_for="${WAIT_READY_POLL_SECONDS}"
        if ((remaining < WAIT_READY_POLL_SECONDS)); then
            sleep_for="${remaining}"
        fi
        sleep "${sleep_for}"
    done
}

step_logs() {
    echo "=== local logs (tail ${LOG_TAIL}) ==="
    (cd "${PROJECT}" && nitrum local logs --tail "${LOG_TAIL}")
}

step_tests() {
    command -v npm >/dev/null 2>&1 || {
        echo "error: npm not found" >&2
        exit 1
    }
    export ENCLAVE_URL="${ENCLAVE_URL:-https://127.0.0.1:443}"
    export ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"
    echo "=== npm test (ENCLAVE_URL=${ENCLAVE_URL}) ==="
    (cd "${PROJECT}" && npm install && npm test)
}

step_down() {
    echo "=== local down ==="
    (cd "${PROJECT}" && nitrum local down)
    STACK_UP=0
}

trap cleanup EXIT

step_init
pin_sdk_git
preclean_stack
step_up
step_wait_ready
step_logs
step_tests
step_down
echo "=== local.sh finished ==="
