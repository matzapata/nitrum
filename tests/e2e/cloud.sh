#!/usr/bin/env bash
# Linear workflow:
#   init → build → cloud env set → cloud deploy → wait active → cloud logs → npm tests (NLB URL) → cloud destroy → cloud env delete
#
# Environment (see tests/e2e/.env.example):
#   NITRUM_BIN               optional override command; default:
#                            cargo run -q ... (quiet compile; see NITRUM_E2E_RUST_LOG)
#   NITRUM_E2E_RUST_LOG      tracing filter for default cargo path (default: warn,cli=info)
#   NITRUM_E2E_PARENT_DIR, NITRUM_E2E_INIT_NAME, PROJECT
#   AWS_REGION or AWS_DEFAULT_REGION
#   NITRUM_E2E_ENV_KEY / NITRUM_E2E_ENV_VALUE   for `cloud env set` (defaults: DEMO / hello)
#   NITRUM_E2E_STACK_NAME                       optional; default: nitrum-<project.name from init> = nitrum-${NITRUM_E2E_INIT_NAME}
#   ENCLAVE_URL                         optional; if set, skips stack lookup for tests
#   ENCLAVE_TLS_INSECURE                        default 1 (self-signed / hostname mismatch on NLB DNS)
#   NITRUM_CLOUD_WAIT_ACTIVE_TIMEOUT_SECONDS    default 300 (5 minutes)
#   NITRUM_CLOUD_WAIT_ACTIVE_POLL_SECONDS       default 5
#   NITRUM_CLOUD_LOGS_SINCE_MINUTES             default 15
#
# Runtime images come from the project's nitrum.toml `[runtime]` section, optionally
# overridden via NITRUM_RUNTIME_DATA_PLANE_IMAGE / NITRUM_RUNTIME_CONTROL_PLANE_IMAGE /
# NITRUM_RUNTIME_NITRO_CLI_IMAGE (see CONTRIBUTING.md for docker buildx bake snippets).
# This script does not build or push images.
#
# Usage: from repo root, `./tests/e2e/cloud.sh`
# The generated workspace under `${NITRUM_E2E_PARENT_DIR}/${NITRUM_E2E_INIT_NAME}` is recreated on each run.
# On EXIT (success, failure, or cancel), destroys the stack and deletes the test env key if they were created.

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
AWS_REGION="${AWS_REGION:-${AWS_DEFAULT_REGION:-}}"
ENV_KEY="${NITRUM_E2E_ENV_KEY:-DEMO}"
ENV_VALUE="${NITRUM_E2E_ENV_VALUE:-hello}"
STACK_NAME="${NITRUM_E2E_STACK_NAME:-nitrum-${NAME}}"
LOGS_SINCE="${NITRUM_CLOUD_LOGS_SINCE_MINUTES:-15}"
WAIT_ACTIVE_TIMEOUT_SECONDS="${NITRUM_CLOUD_WAIT_ACTIVE_TIMEOUT_SECONDS:-300}"
WAIT_ACTIVE_POLL_SECONDS="${NITRUM_CLOUD_WAIT_ACTIVE_POLL_SECONDS:-5}"
STACK_DEPLOYED=0
ENV_SET=0

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
    if [[ "${STACK_DEPLOYED}" == "1" && -f "${PROJECT}/nitrum.toml" ]]; then
        echo "=== cleanup: cloud destroy ==="
        nitrum cloud destroy --force --path "${PROJECT}" || true
        STACK_DEPLOYED=0
    fi
    if [[ "${ENV_SET}" == "1" && -f "${PROJECT}/nitrum.toml" ]]; then
        echo "=== cleanup: cloud env delete ${ENV_KEY} ==="
        nitrum cloud env --path "${PROJECT}" delete "${ENV_KEY}" --force || true
        ENV_SET=0
    fi
}

step_build() {
    echo "=== build ==="
    (cd "${PROJECT}" && nitrum build)
}

step_env_set() {
    echo "=== cloud env set ${ENV_KEY} ==="
    nitrum cloud env --path "${PROJECT}" set "${ENV_KEY}" "${ENV_VALUE}"
    ENV_SET=1
}

step_deploy() {
    echo "=== cloud deploy ==="
    nitrum cloud deploy --force --path "${PROJECT}"
    STACK_DEPLOYED=1
}

step_wait_active() {
    command -v curl >/dev/null 2>&1 || {
        echo "error: curl not found (needed to wait for ENCLAVE_URL readiness)" >&2
        exit 1
    }

    local base
    base="$(resolve_base_url)" || exit 1
    export ENCLAVE_URL="${base}"
    export ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"

    echo "=== wait active (timeout ${WAIT_ACTIVE_TIMEOUT_SECONDS}s): ${ENCLAVE_URL} ==="

    local deadline now remaining sleep_for
    deadline=$((SECONDS + WAIT_ACTIVE_TIMEOUT_SECONDS))

    local -a curl_args
    curl_args=(-sS -o /dev/null --connect-timeout 5 --max-time 10)
    if [[ "${ENCLAVE_TLS_INSECURE}" == "1" ]]; then
        curl_args+=(-k)
    fi

    while true; do
        if curl "${curl_args[@]}" "${ENCLAVE_URL}"; then
            echo "=== enclave is responsive ==="
            return 0
        fi

        now=${SECONDS}
        if ((now >= deadline)); then
            echo "error: enclave did not become responsive within ${WAIT_ACTIVE_TIMEOUT_SECONDS}s (${ENCLAVE_URL})" >&2
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

step_cloud_logs() {
    echo "=== cloud logs (since ${LOGS_SINCE} min, no follow) ==="
    nitrum cloud logs --path "${PROJECT}" --since-minutes "${LOGS_SINCE}"
}

resolve_base_url() {
    if [[ -n "${ENCLAVE_URL:-}" ]]; then
        echo "${ENCLAVE_URL%/}"
        return 0
    fi
    if [[ -z "${AWS_REGION}" ]]; then
        echo "error: set AWS_REGION (or ENCLAVE_URL)" >&2
        return 1
    fi
    command -v aws >/dev/null 2>&1 || {
        echo "error: aws CLI not found (needed to read NLBDnsName)" >&2
        return 1
    }
    local nlb
    nlb="$(aws cloudformation describe-stacks \
        --stack-name "${STACK_NAME}" \
        --region "${AWS_REGION}" \
        --query 'Stacks[0].Outputs[?OutputKey==`NLBDnsName`].OutputValue | [0]' \
        --output text)"
    if [[ -z "${nlb}" || "${nlb}" == "None" ]]; then
        echo "error: stack ${STACK_NAME} has no NLBDnsName output" >&2
        return 1
    fi
    echo "https://${nlb}"
}

step_tests() {
    command -v npm >/dev/null 2>&1 || {
        echo "error: npm not found" >&2
        exit 1
    }
    local base
    base="$(resolve_base_url)" || exit 1
    export ENCLAVE_URL="${base}"
    export ENCLAVE_TLS_INSECURE="${ENCLAVE_TLS_INSECURE:-1}"
    echo "=== npm test (ENCLAVE_URL=${ENCLAVE_URL}) ==="
    (cd "${PROJECT}" && npm install && npm test)
}

step_destroy() {
    echo "=== cloud destroy ==="
    nitrum cloud destroy --force --path "${PROJECT}"
    STACK_DEPLOYED=0
}

step_env_delete() {
    echo "=== cloud env delete ${ENV_KEY} ==="
    nitrum cloud env --path "${PROJECT}" delete "${ENV_KEY}" --force
    ENV_SET=0
}

trap cleanup EXIT

step_init
pin_sdk_git
step_build
step_env_set
step_deploy
step_wait_active
step_cloud_logs
step_tests
step_destroy
step_env_delete
echo "=== cloud.sh finished ==="
