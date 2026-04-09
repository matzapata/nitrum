#!/usr/bin/env bash
# Linear workflow:
#   init → build → cloud env set → cloud deploy → cloud logs → npm tests (NLB URL) → cloud destroy → cloud env delete
#
# Environment (see tests/e2e/.env.example):
#   NITRUM_BIN               optional override command; default:
#                            cargo run --manifest-path <repo>/Cargo.toml --bin nitrum
#   NITRUM_E2E_PARENT_DIR, NITRUM_E2E_INIT_NAME, PROJECT
#   AWS_REGION or AWS_DEFAULT_REGION
#   NITRUM_E2E_ENV_KEY / NITRUM_E2E_ENV_VALUE   for `cloud env set` (defaults: DEMO / hello)
#   NITRUM_E2E_STACK_NAME                       optional; default: nitrum-<project.name from init> = nitrum-${NITRUM_E2E_INIT_NAME}
#   ENCLAVE_URL                         optional; if set, skips stack lookup for tests
#   ENCLAVE_TLS_INSECURE                        default 1 (self-signed / hostname mismatch on NLB DNS)
#   NITRUM_CLOUD_LOGS_SINCE_MINUTES             default 15
#
# Usage: from repo root, `./tests/e2e/cloud.sh`

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
AWS_REGION="${AWS_REGION:-${AWS_DEFAULT_REGION:-}}"
ENV_KEY="${NITRUM_E2E_ENV_KEY:-DEMO}"
ENV_VALUE="${NITRUM_E2E_ENV_VALUE:-hello}"
STACK_NAME="${NITRUM_E2E_STACK_NAME:-nitrum-${NAME}}"
LOGS_SINCE="${NITRUM_CLOUD_LOGS_SINCE_MINUTES:-15}"

nitrum() {
    local -a cmd
    if [[ -n "${NITRUM_BIN:-}" ]]; then
        # shellcheck disable=SC2206
        cmd=(${NITRUM_BIN})
    else
        cmd=(cargo run --manifest-path "${REPO_ROOT}/Cargo.toml" --bin nitrum)
    fi
    "${cmd[@]}" -- "$@"
}

step_init() {
    mkdir -p "${PARENT}"
    if [[ -f "${PROJECT}/nitrum.toml" ]]; then
        echo "=== init: skip (found nitrum.toml) ==="
        return 0
    fi
    if [[ -e "${PROJECT}" ]]; then
        echo "error: ${PROJECT} exists but has no nitrum.toml" >&2
        exit 1
    fi
    echo "=== init: ${NAME} in ${PARENT} ==="
    (cd "${PARENT}" && nitrum init "${NAME}")
}

step_build() {
    echo "=== build ==="
    (cd "${PROJECT}" && nitrum build)
}

step_env_set() {
    echo "=== cloud env set ${ENV_KEY} ==="
    nitrum cloud env --path "${PROJECT}" set "${ENV_KEY}" "${ENV_VALUE}"
}

step_deploy() {
    echo "=== cloud deploy ==="
    nitrum cloud deploy --force --path "${PROJECT}"
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
}

step_env_delete() {
    echo "=== cloud env delete ${ENV_KEY} ==="
    nitrum cloud env --path "${PROJECT}" delete "${ENV_KEY}" --force
}

step_init
step_build
step_env_set
step_deploy
step_cloud_logs
step_tests
step_destroy
step_env_delete
echo "=== cloud.sh finished ==="
