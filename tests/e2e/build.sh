#!/usr/bin/env bash
# Linear workflow: nitrum init → nitrum build → nitrum describe (default EIF).
#
# Environment (see tests/e2e/.env.example):
#   NITRUM_BIN               optional override command; default:
#                            cargo run --manifest-path <repo>/Cargo.toml --bin nitrum
#   NITRUM_E2E_PARENT_DIR    parent directory for `nitrum init <name>` (default: <repo>/target/nitrum-e2e-workspace)
#   NITRUM_E2E_INIT_NAME     project folder / project.name (default: nitrum-e2e-demo)
#
# If ${PARENT}/${NAME}/nitrum.toml already exists, init is skipped.
#
# Usage: from repo root, `./tests/e2e/build.sh`

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
        echo "error: ${PROJECT} exists but has no nitrum.toml; remove it or change NITRUM_E2E_INIT_NAME" >&2
        exit 1
    fi
    echo "=== init: ${NAME} in ${PARENT} ==="
    (cd "${PARENT}" && nitrum init "${NAME}")
}

step_build() {
    echo "=== build ==="
    (cd "${PROJECT}" && nitrum build)
}

step_describe() {
    echo "=== describe ==="
    (cd "${PROJECT}" && nitrum describe)
}

step_init
step_build
step_describe
echo "=== build.sh finished ==="
