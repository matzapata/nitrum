DOCKERHUB_USER := "matzapata"
CONTROL_PLANE_TAG := "matzapata/control-plane:latest"
CONTROL_PLANE_DEV_TAG := "matzapata/control-plane:dev"
DATA_PLANE_TAG := "matzapata/data-plane:latest"
DATA_PLANE_DEV_TAG := "matzapata/data-plane:dev"

# ── Dev builds (TCP transport, no enclave feature) ───────────────────────────

build-control-plane-dev:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile -t {{CONTROL_PLANE_DEV_TAG}} .

rebuild-control-plane-dev:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile --no-cache -t {{CONTROL_PLANE_DEV_TAG}} .

build-data-plane-dev:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile -t {{DATA_PLANE_DEV_TAG}} .

rebuild-data-plane-dev:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile --no-cache -t {{DATA_PLANE_DEV_TAG}} .

build-dev: build-control-plane-dev build-data-plane-dev

rebuild-dev: rebuild-control-plane-dev rebuild-data-plane-dev

# ── Enclave builds (VSock transport, enclave feature enabled, AMD64) ──────────

build-control-plane:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile --build-arg FEATURES=enclave -t {{CONTROL_PLANE_TAG}} .

rebuild-control-plane:
    docker build --platform linux/amd64 -f docker/control-plane.dockerfile --no-cache --build-arg FEATURES=enclave -t {{CONTROL_PLANE_TAG}} .

build-data-plane:

rebuild-data-plane:
    docker build --platform linux/amd64 -f docker/data-plane.dockerfile --no-cache --build-arg FEATURES=enclave -t {{DATA_PLANE_TAG}} .

build: build-control-plane build-data-plane

rebuild: rebuild-control-plane rebuild-data-plane

# ── Docker Hub publish ────────────────────────────────────────────────────────

push-control-plane: rebuild-control-plane
    docker push {{CONTROL_PLANE_TAG}}

push-data-plane: rebuild-data-plane
    docker push {{DATA_PLANE_TAG}}

push: push-control-plane push-data-plane

