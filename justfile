dockerhub_user := "matzapata"

# ── dev tools ─────────────────────────────────────────

check:
    cargo check --all-targets

lint:
    cargo clippy --all-targets --all-features

format:
    cargo fmt --all

# ── Docker builds ───────────────────────────────────

build-nitro-cli tag="latest" no_cache="":
    docker build \
        --platform linux/amd64 \
        -f infra/docker/nitro-cli.dockerfile \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-nitro-cli:{{tag}} \
        .

push-nitro-cli tag="latest":
    docker push {{dockerhub_user}}/nitrum-nitro-cli:{{tag}}

build-control-plane tag="latest" no_cache="":
    docker build \
        --platform linux/amd64 \
        {{ if tag == "dev" { "-f infra/docker/control-plane.dockerfile" } else { "-f infra/docker/control-plane.dockerfile --build-arg FEATURES=enclave" } }} \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-control-plane:{{tag}} \
        .

push-control-plane tag="latest":
    docker push {{dockerhub_user}}/nitrum-control-plane:{{tag}}

build-data-plane tag="latest" no_cache="":
    docker build \
        --platform linux/amd64 \
        -f infra/docker/data-plane.dockerfile \
        --build-arg FEATURES=enclave \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-data-plane:{{tag}} \
        .

build-dev-data-plane no_cache="":
    docker build \
        --platform linux/amd64 \
        -f infra/docker/data-plane.dockerfile \
        --build-arg FEATURES=pebble \
        {{ if no_cache == "true" { "--no-cache" } else { "" } }} \
        -t {{ dockerhub_user }}/nitrum-data-plane:dev \
        .

push-data-plane tag="latest":
    docker push {{dockerhub_user}}/nitrum-data-plane:{{tag}}

# ── Push ──────────────────────────────────────────────

push-all tag="latest":
    just push-control-plane {{tag}}
    just push-data-plane {{tag}}
    just push-nitro-cli {{tag}} 
    
