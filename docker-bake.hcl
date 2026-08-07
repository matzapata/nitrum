# Build Nitrum platform images with docker buildx bake.
#
# Variables (override via env or `--set`):
#   IMAGE_PREFIX     registry/org prefix (default: nitrum)
#   TAG              image tag (default: dev)
#   GIT_SHA          git revision label (default: unknown)
#   DOCKER_PLATFORM  target platform (default: linux/amd64)
#
# Examples:
#   # Local pebble data-plane (loads into the Docker daemon; no --push):
#   IMAGE_PREFIX=docker.io/matzapata TAG=dev GIT_SHA=$(git rev-parse HEAD) \
#     docker buildx bake data-plane-local
#
#   # Cloud / prod images (push to a registry):
#   IMAGE_PREFIX=docker.io/matzapata TAG=$(git rev-parse HEAD) GIT_SHA=$(git rev-parse HEAD) \
#     docker buildx bake --push control-plane data-plane nitro-cli
#
# Then point the CLI at the same tags via:
#   export NITRUM_RUNTIME_DATA_PLANE_IMAGE="${IMAGE_PREFIX}/data-plane:${TAG}"
#   export NITRUM_RUNTIME_CONTROL_PLANE_IMAGE="${IMAGE_PREFIX}/control-plane:${TAG}"
#   export NITRUM_RUNTIME_NITRO_CLI_IMAGE="${IMAGE_PREFIX}/nitro-cli:${TAG}"
#
# `nitrum local up` automatically appends `-local` to the resolved data_plane tag.

variable "IMAGE_PREFIX" {
  default = "nitrum"
}

variable "TAG" {
  default = "dev"
}

variable "GIT_SHA" {
  default = "unknown"
}

variable "DOCKER_PLATFORM" {
  default = "linux/amd64"
}

group "default" {
  targets = ["control-plane", "data-plane", "data-plane-local", "nitro-cli"]
}

target "control-plane" {
  context    = "."
  dockerfile = "crates/control-plane/Dockerfile"
  platforms  = ["${DOCKER_PLATFORM}"]
  args = {
    GIT_SHA = "${GIT_SHA}"
    VERSION = "${TAG}"
  }
  tags = ["${IMAGE_PREFIX}/control-plane:${TAG}"]
}

target "data-plane" {
  context    = "."
  dockerfile = "crates/data-plane/Dockerfile"
  platforms  = ["${DOCKER_PLATFORM}"]
  args = {
    FEATURES = "enclave"
    GIT_SHA  = "${GIT_SHA}"
    VERSION  = "${TAG}"
  }
  tags = ["${IMAGE_PREFIX}/data-plane:${TAG}"]
}

target "data-plane-local" {
  inherits = ["data-plane"]
  args = {
    FEATURES = "pebble"
    GIT_SHA  = "${GIT_SHA}"
    VERSION  = "${TAG}"
  }
  tags   = ["${IMAGE_PREFIX}/data-plane:${TAG}-local"]
}

target "nitro-cli" {
  context    = "."
  dockerfile = "crates/cli/nitro-cli.dockerfile"
  platforms  = ["${DOCKER_PLATFORM}"]
  args = {
    GIT_SHA = "${GIT_SHA}"
    VERSION = "${TAG}"
  }
  tags = ["${IMAGE_PREFIX}/nitro-cli:${TAG}"]
}
