# ── Dev builds (TCP transport, no enclave feature) ───────────────────────────

build-control-plane-dev:
    docker build -f docker/control-plane.dockerfile -t nitrum/control-plane:dev .

build-data-plane-dev:
    docker build -f docker/data-plane.dockerfile -t nitrum/data-plane:dev .

build-dev: build-control-plane-dev build-data-plane-dev

# ── Enclave builds (VSock transport, enclave feature enabled) ─────────────────

build-control-plane:
    docker build -f docker/control-plane.dockerfile --build-arg FEATURES=enclave -t nitrum/control-plane:latest .

build-data-plane:
    docker build -f docker/data-plane.dockerfile --build-arg FEATURES=enclave -t nitrum/data-plane:latest .

build: build-control-plane build-data-plane
