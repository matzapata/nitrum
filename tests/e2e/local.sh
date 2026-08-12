#!/usr/bin/env bash
# Linear workflow: init → enable Pebble ACME → local up → wait ready → wait ACME cert → local logs → npm integration tests → local down.
#
# Environment (see tests/e2e/.env.example):
#   NITRUM_BIN               optional override command; default:
#                            cargo run -q ... (quiet compile; see NITRUM_E2E_RUST_LOG)
#   NITRUM_E2E_RUST_LOG      tracing filter for default cargo path (default: warn,cli=info)
#   NITRUM_E2E_PARENT_DIR, NITRUM_E2E_INIT_NAME, PROJECT path
#   ENCLAVE_URL              default: https://nitrum.localhost:443
#   ENCLAVE_TLS_INSECURE     default: 1 (Pebble/minica is not in the system trust store)
#   NITRUM_LOCAL_LOGS_TAIL   passed to `nitrum local logs --tail` (default: 80)
#   NITRUM_LOCAL_WAIT_READY_TIMEOUT_SECONDS  default 60
#   NITRUM_LOCAL_WAIT_ACME_TIMEOUT_SECONDS   default 180
#   NITRUM_LOCAL_WAIT_READY_POLL_SECONDS     default 1
#
# Runtime images come from the project's nitrum.toml `[runtime]` section, optionally
# overridden via NITRUM_RUNTIME_DATA_PLANE_IMAGE (and siblings). `nitrum local up`
# automatically appends `-local` to the resolved data_plane tag. Build images with
# `docker buildx bake` — see CONTRIBUTING.md. This script does not build images.
#
# Add `127.0.0.1 nitrum.localhost` to /etc/hosts if https://nitrum.localhost does not resolve
# (required on some Windows setups; modern macOS/Linux usually resolve `.localhost` to 127.0.0.1).
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
WAIT_ACME_TIMEOUT_SECONDS="${NITRUM_LOCAL_WAIT_ACME_TIMEOUT_SECONDS:-180}"
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

resolve_enclave_endpoint() {
    export ENCLAVE_URL="${ENCLAVE_URL:-https://nitrum.localhost:443}"
    export ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"

    local hostport="${ENCLAVE_URL#https://}"
    hostport="${hostport#http://}"
    hostport="${hostport%/}"

    TLS_HOST="${hostport%%:*}"
    if [[ "${hostport}" == *:* ]]; then
        TLS_PORT="${hostport##*:}"
    else
        TLS_PORT="443"
    fi
    TLS_SNI="${TLS_HOST}"
}

curl_tls_args() {
    CURL_TLS_ARGS=(-sS -o /dev/null -w '%{http_code}' --connect-timeout 2 --max-time 5)
    if [[ "${ENCLAVE_TLS_INSECURE}" == "1" ]]; then
        CURL_TLS_ARGS+=(-k)
    fi
}

fetch_tls_cert_meta() {
    # Leaf identity may live only in SAN (Pebble often issues an empty Subject DN).
    local meta
    meta="$(
        echo | openssl s_client -connect "${TLS_HOST}:${TLS_PORT}" -servername "${TLS_SNI}" 2>/dev/null \
            | openssl x509 -noout -issuer -subject -ext subjectAltName 2>/dev/null || true
    )"
    TLS_CERT_ISSUER="$(printf '%s\n' "${meta}" | sed -n 's/^issuer=//p' | head -1)"
    TLS_CERT_SUBJECT="$(printf '%s\n' "${meta}" | sed -n 's/^subject=//p' | head -1)"
    # Flatten extension text so we can substring-match DNS:<sni>.
    TLS_CERT_SAN="$(printf '%s\n' "${meta}" | tr '\n' ' ')"
}

# Expects TLS_CERT_* from fetch_tls_cert_meta and TLS_SNI from resolve_enclave_endpoint.
#
# Pebble leaves are issued by "Pebble Intermediate CA …" (minica is the root, not the
# leaf issuer). Reject the bootstrap rcgen self-signed cert.
is_pebble_acme_cert() {
    local issuer_lc subject_lc san_lc sni_lc
    [[ -n "${TLS_CERT_ISSUER:-}" ]] || return 1
    issuer_lc="$(printf '%s' "${TLS_CERT_ISSUER}" | tr '[:upper:]' '[:lower:]')"
    subject_lc="$(printf '%s' "${TLS_CERT_SUBJECT:-}" | tr '[:upper:]' '[:lower:]')"
    # OpenSSL may print `DNS:host` or `DNS: host`.
    san_lc="$(printf '%s' "${TLS_CERT_SAN:-}" | tr '[:upper:]' '[:lower:]' | sed 's/dns: */dns:/g')"
    sni_lc="$(printf '%s' "${TLS_SNI}" | tr '[:upper:]' '[:lower:]')"

    [[ "${issuer_lc}" == *pebble* ]] || return 1
    [[ "${issuer_lc}" != *rcgen* ]] || return 1
    # Not the bootstrap self-signed leaf.
    [[ -z "${TLS_CERT_SUBJECT:-}" || "${TLS_CERT_ISSUER}" != "${TLS_CERT_SUBJECT}" ]] || return 1
    # Domain must appear in Subject CN and/or SAN.
    [[ "${subject_lc}" == *"${sni_lc}"* || "${san_lc}" == *"dns:${sni_lc}"* ]] || return 1
    return 0
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

step_enable_acme() {
    local nitrum_toml="${PROJECT}/nitrum.toml"
    if [[ ! -f "${nitrum_toml}" ]]; then
        echo "error: missing ${nitrum_toml}" >&2
        exit 1
    fi

    echo "=== enable Pebble ACME in nitrum.toml ==="
    local tmp
    tmp="$(mktemp)"
    awk '
        /^\[tls_termination\]/ { in_tls = 1; print; next }
        /^\[/ { in_tls = 0 }
        in_tls && /^acme = / { print "acme = true"; next }
        { print }
    ' "${nitrum_toml}" >"${tmp}"
    mv "${tmp}" "${nitrum_toml}"

    if ! awk '
        /^\[tls_termination\]/ { in_tls = 1; next }
        /^\[/ { in_tls = 0 }
        in_tls && /^acme = true$/ { found = 1 }
        END { exit !found }
    ' "${nitrum_toml}"; then
        echo "error: failed to set [tls_termination].acme = true in ${nitrum_toml}" >&2
        exit 1
    fi
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

    resolve_enclave_endpoint
    local status_url="${ENCLAVE_URL%/}/.well-known/enclave/status"

    echo "=== wait ready (timeout ${WAIT_READY_TIMEOUT_SECONDS}s): ${status_url} ==="

    local deadline now remaining sleep_for http_code
    deadline=$((SECONDS + WAIT_READY_TIMEOUT_SECONDS))

    curl_tls_args

    while true; do
        http_code="$(curl "${CURL_TLS_ARGS[@]}" "${status_url}" || true)"
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

# Wait until ingress serves a Pebble-issued leaf (not the bootstrap self-signed cert).
step_wait_acme() {
    command -v openssl >/dev/null 2>&1 || {
        echo "error: openssl not found (needed to verify Pebble ACME certificate)" >&2
        exit 1
    }

    resolve_enclave_endpoint

    echo "=== wait Pebble ACME cert (timeout ${WAIT_ACME_TIMEOUT_SECONDS}s): ${TLS_SNI}:${TLS_PORT} ==="

    local deadline now remaining sleep_for
    deadline=$((SECONDS + WAIT_ACME_TIMEOUT_SECONDS))

    while true; do
        fetch_tls_cert_meta
        if is_pebble_acme_cert; then
            echo "=== Pebble ACME certificate active ==="
            echo "  subject: ${TLS_CERT_SUBJECT:-"(empty)"}"
            echo "  issuer:  ${TLS_CERT_ISSUER}"
            return 0
        fi

        now=${SECONDS}
        if ((now >= deadline)); then
            echo "error: Pebble ACME certificate not active within ${WAIT_ACME_TIMEOUT_SECONDS}s" >&2
            echo "  last subject: ${TLS_CERT_SUBJECT:-none}" >&2
            echo "  last issuer:  ${TLS_CERT_ISSUER:-none}" >&2
            echo "  last san:     ${TLS_CERT_SAN:-none}" >&2
            exit 1
        fi

        remaining=$((deadline - now))
        echo "waiting for Pebble ACME cert... (subject=${TLS_CERT_SUBJECT:-none}; issuer=${TLS_CERT_ISSUER:-none}; ${remaining}s remaining)"
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
    resolve_enclave_endpoint
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
step_enable_acme
preclean_stack
step_up
step_wait_ready
step_wait_acme
step_logs
step_tests
step_down
