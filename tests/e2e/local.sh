#!/usr/bin/env bash
# Linear workflow: init → local up → local logs → npm integration tests → local down.
#
# Environment (see tests/e2e/.env.example):
#   NITRUM_BIN               optional override command; default:
#                            cargo run -q ... (quiet compile; see NITRUM_E2E_RUST_LOG)
#   NITRUM_E2E_RUST_LOG      tracing filter for default cargo path (default: warn,cli=info)
#   NITRUM_E2E_PARENT_DIR, NITRUM_E2E_INIT_NAME, PROJECT path
#   ENCLAVE_URL              default: https://nitrum.local (override for http://127.0.0.1:443 if you prefer)
#   ENCLAVE_TLS_INSECURE     default: 1
#   NITRUM_LOCAL_LOGS_TAIL   passed to `nitrum local logs --tail` (default: 80)
#   NITRUM_LOCAL_DATA_PLANE_IMAGE
#                            optional; if unset, e2e builds a pebble data-plane image once and uses it
#   NITRUM_E2E_REBUILD_DATA_PLANE
#                            if non-empty, force `docker build` even when the local tag exists
#
# Add 127.0.0.1 nitrum.local to /etc/hosts if needed.
#
# Usage: from repo root, `./tests/e2e/local.sh`
# The generated workspace under `${NITRUM_E2E_PARENT_DIR}/${NITRUM_E2E_INIT_NAME}` is recreated on each run.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

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
E2E_DATA_PLANE_TAG="${NITRUM_E2E_DATA_PLANE_TAG:-nitrum-e2e-data-plane:local}"
STACK_UP=0

ensure_local_data_plane_image() {
    if [[ -n "${NITRUM_LOCAL_DATA_PLANE_IMAGE:-}" ]]; then
        echo "=== data-plane: using NITRUM_LOCAL_DATA_PLANE_IMAGE=${NITRUM_LOCAL_DATA_PLANE_IMAGE} ==="
        return 0
    fi
    if [[ -n "${NITRUM_E2E_REBUILD_DATA_PLANE:-}" ]] || ! docker image inspect "${E2E_DATA_PLANE_TAG}" >/dev/null 2>&1; then
        echo "=== data-plane: docker build ${E2E_DATA_PLANE_TAG} (pebble, linux/amd64) ==="
        docker build --platform linux/amd64 \
            -f "${REPO_ROOT}/crates/data-plane/Dockerfile" \
            --build-arg FEATURES=pebble \
            -t "${E2E_DATA_PLANE_TAG}" \
            "${REPO_ROOT}"
    else
        echo "=== data-plane: reuse image ${E2E_DATA_PLANE_TAG} (set NITRUM_E2E_REBUILD_DATA_PLANE=1 to rebuild) ==="
    fi
    export NITRUM_LOCAL_DATA_PLANE_IMAGE="${E2E_DATA_PLANE_TAG}"
}

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
ensure_local_data_plane_image
preclean_stack
step_up
step_logs
step_tests
step_down
echo "=== local.sh finished ==="
