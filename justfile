dockerhub_user := "matzapata"
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
        -t {{ dockerhub_user }}/nitrum-nitro-cli:{{tag}} \
        .

push-nitro-cli:
    docker push {{dockerhub_user}}/nitrum-nitro-cli:{{tag}}

# ── Control Plane ───────────────────────────────────

build-control-plane no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/control-plane/Dockerfile \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-control-plane:{{tag}} \
        .

push-control-plane:
    docker push {{dockerhub_user}}/nitrum-control-plane:{{tag}}

# ── Data Plane ───────────────────────────────────

build-data-plane no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/data-plane/Dockerfile \
        --build-arg FEATURES=enclave \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-data-plane:{{tag}} \
        .


push-data-plane:
    docker push {{dockerhub_user}}/nitrum-data-plane:{{tag}}

# ── Data Plane Dev ───────────────────────────────────

build-data-plane-dev no_cache="":
    docker build \
        --platform linux/amd64 \
        -f crates/data-plane/Dockerfile \
        --build-arg FEATURES=pebble \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-data-plane:{{tag}}-dev \
        .

push-data-plane-dev:
    docker push {{dockerhub_user}}/nitrum-data-plane:{{tag}}-dev



    
