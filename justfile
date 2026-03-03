build-control-plane:
    docker build -f docker/control-plane.dockerfile -t nitrum/control-plane:latest .

build-data-plane:
    docker build -f docker/data-plane.dockerfile -t nitrum/data-plane:latest .

build: build-control-plane build-data-plane
