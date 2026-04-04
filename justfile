dockerhub_user := "matzapata"
tag := "v0.1.1"

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
        -t {{ dockerhub_user }}/nitrum-nitro-cli:{{tag}} \
        -t {{ dockerhub_user }}/nitrum-nitro-cli:latest \
        .

push-nitro-cli:
    docker push {{dockerhub_user}}/nitrum-nitro-cli:{{tag}}
    docker push {{dockerhub_user}}/nitrum-nitro-cli:latest

# ── Control Plane ───────────────────────────────────

build-control-plane no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/control-plane/Dockerfile \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-control-plane:{{tag}} \
        -t {{ dockerhub_user }}/nitrum-control-plane:latest \
        .

push-control-plane:
    docker push {{dockerhub_user}}/nitrum-control-plane:{{tag}}
    docker push {{dockerhub_user}}/nitrum-control-plane:latest

# ── Data Plane ───────────────────────────────────

build-data-plane no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/data-plane/Dockerfile \
        --build-arg FEATURES=enclave \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-data-plane:{{tag}} \
        -t {{ dockerhub_user }}/nitrum-data-plane:latest \
        .


push-data-plane:
    docker push {{dockerhub_user}}/nitrum-data-plane:{{tag}}
    docker push {{dockerhub_user}}/nitrum-data-plane:latest

# ── Data Plane Dev ───────────────────────────────────

build-data-plane-dev no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/data-plane/Dockerfile \
        --build-arg FEATURES=pebble \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-data-plane:{{tag}}-dev \
        -t {{ dockerhub_user }}/nitrum-data-plane:latest-dev \
        .

push-data-plane-dev:
    docker push {{dockerhub_user}}/nitrum-data-plane:{{tag}}-dev
    docker push {{dockerhub_user}}/nitrum-data-plane:latest-dev
