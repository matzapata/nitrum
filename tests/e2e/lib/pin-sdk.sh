# Shared helper: pin the scaffold's git `sdk` dep to this checkout's rev.
# Requires REPO_ROOT and PROJECT. Sourced by local.sh / cloud.sh / build.sh.

pin_sdk_git() {
    local cargo_toml="${PROJECT}/Cargo.toml"
    local branch rev short_rev new_line
    local git_url="https://github.com/matzapata/nitrum.git"
    local package="nitrum-sdk"

    if [[ ! -f "${cargo_toml}" ]]; then
        echo "error: pin_sdk_git: missing ${cargo_toml}" >&2
        exit 1
    fi

    branch="$(git -C "${REPO_ROOT}" rev-parse --abbrev-ref HEAD)"
    rev="$(git -C "${REPO_ROOT}" rev-parse HEAD)"
    short_rev="$(git -C "${REPO_ROOT}" rev-parse --short HEAD)"

    new_line="sdk = { git = \"${git_url}\", package = \"${package}\", rev = \"${rev}\" }"

    if ! grep -qE '^sdk = \{ git = "https://github.com/matzapata/nitrum.git"' "${cargo_toml}"; then
        echo "error: pin_sdk_git: expected sdk git dependency in ${cargo_toml}" >&2
        exit 1
    fi

    local tmp
    tmp="$(mktemp)"
    # Replace any existing sdk git dep line (template develop pin or a prior e2e pin).
    while IFS= read -r line || [[ -n "${line}" ]]; do
        if [[ "${line}" == sdk\ =\ \{\ git\ =\ \"https://github.com/matzapata/nitrum.git\"* ]]; then
            printf '%s\n' "${new_line}"
        else
            printf '%s\n' "${line}"
        fi
    done <"${cargo_toml}" >"${tmp}"
    mv "${tmp}" "${cargo_toml}"

    if ! grep -qF "${new_line}" "${cargo_toml}"; then
        echo "error: pin_sdk_git: failed to rewrite sdk dependency in ${cargo_toml}" >&2
        exit 1
    fi

    echo "=== pin sdk: package=${package} branch=${branch} rev=${rev} (${short_rev}) ==="
    echo "=== pin sdk: ${new_line} ==="
}
