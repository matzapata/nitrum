tag_prefix := "matzapata/"
tag := "v0.1.0"

# ── dev tools ─────────────────────────────────────────

check:
    cargo check --all-targets

lint:
    cargo clippy --all-targets --all-features

format:
    cargo fmt --all

# ── Nitro CLI ───────────────────────────────────

build-nitro-cli no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/cli/Dockerfile \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ tag_prefix }}nitro-cli:{{tag}} \
        -t {{ tag_prefix }}nitro-cli:latest \
        .

push-nitro-cli:
    docker push {{tag_prefix}}nitro-cli:{{tag}}
    docker push {{tag_prefix}}nitro-cli:latest

# ── Control Plane ───────────────────────────────────

build-control-plane no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/control-plane/Dockerfile \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ tag_prefix }}control-plane:{{tag}} \
        -t {{ tag_prefix }}control-plane:latest \
        .

push-control-plane:
    docker push {{tag_prefix}}control-plane:{{tag}}
    docker push {{tag_prefix}}control-plane:latest

# ── Data Plane ───────────────────────────────────

build-data-plane no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/data-plane/Dockerfile \
        --build-arg FEATURES=enclave \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ tag_prefix }}data-plane:{{tag}} \
        -t {{ tag_prefix }}data-plane:latest \
        .


push-data-plane:
    docker push {{ tag_prefix }}data-plane:{{tag}}
    docker push {{ tag_prefix }}data-plane:latest

# ── Data Plane Dev ───────────────────────────────────

build-data-plane-dev no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/data-plane/Dockerfile \
        --build-arg FEATURES=pebble \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ tag_prefix }}data-plane:{{tag}}-dev \
        -t {{ tag_prefix }}data-plane:latest-dev \
        .

push-data-plane-dev:
    docker push {{ tag_prefix }}data-plane:{{tag}}-dev
    docker push {{ tag_prefix }}data-plane:latest-dev
