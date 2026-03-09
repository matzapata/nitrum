DOCKERHUB_USER := "matzapata"

# ── Dev builds (TCP transport, no enclave feature) ───────────────────────────

build-control-plane-dev:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile -t nitrum/control-plane:dev .

rebuild-control-plane-dev:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile --no-cache -t nitrum/control-plane:dev .

build-data-plane-dev:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile -t nitrum/data-plane:dev .

rebuild-data-plane-dev:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile --no-cache -t nitrum/data-plane:dev .

build-dev: build-control-plane-dev build-data-plane-dev

rebuild-dev: rebuild-control-plane-dev rebuild-data-plane-dev

# ── Enclave builds (VSock transport, enclave feature enabled, AMD64) ──────────

build-control-plane:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile --build-arg FEATURES=enclave -t nitrum/control-plane:latest .

rebuild-control-plane:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile --no-cache --build-arg FEATURES=enclave -t nitrum/control-plane:latest .

build-data-plane:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile --build-arg FEATURES=enclave -t nitrum/data-plane:latest .

rebuild-data-plane:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile --no-cache --build-arg FEATURES=enclave -t nitrum/data-plane:latest .

build: build-control-plane build-data-plane

rebuild: rebuild-control-plane rebuild-data-plane

# ── Docker Hub publish ────────────────────────────────────────────────────────

push-control-plane: 
    docker tag nitrum/control-plane:latest {{DOCKERHUB_USER}}/nitrum-control-plane:latest
    docker push {{DOCKERHUB_USER}}/nitrum-control-plane:latest

push-data-plane: 
    docker tag nitrum/data-plane:latest {{DOCKERHUB_USER}}/nitrum-data-plane:latest
    docker push {{DOCKERHUB_USER}}/nitrum-data-plane:latest

push: push-control-plane push-data-plane

