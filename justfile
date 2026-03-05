DOCKERHUB_USER := "matzapata"

# ── Dev builds (TCP transport, no enclave feature) ───────────────────────────

build-control-plane-dev:
    docker build -f docker/control-plane.dockerfile -t nitrum/control-plane:dev .

build-data-plane-dev:
    docker build -f docker/data-plane.dockerfile -t nitrum/data-plane:dev .

build-dev: build-control-plane-dev build-data-plane-dev


# ── Enclave builds (VSock transport, enclave feature enabled, AMD64) ──────────
# Must target linux/amd64 – Nitro enclaves run on x86_64 only.
# Cross-compiling from Apple Silicon requires Docker Desktop with Rosetta/QEMU.

build-control-plane:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile --build-arg FEATURES=enclave -t nitrum/control-plane:latest .

build-data-plane:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile --build-arg FEATURES=enclave -t nitrum/data-plane:latest .

build: build-control-plane build-data-plane


# ── Docker Hub publish ────────────────────────────────────────────────────────

push-control-plane: build-control-plane
    docker tag nitrum/control-plane:latest {{DOCKERHUB_USER}}/nitrum-control-plane:latest
    docker push {{DOCKERHUB_USER}}/nitrum-control-plane:latest

push-data-plane: build-data-plane
    docker tag nitrum/data-plane:latest {{DOCKERHUB_USER}}/nitrum-data-plane:latest
    docker push {{DOCKERHUB_USER}}/nitrum-data-plane:latest

push: push-control-plane push-data-plane
