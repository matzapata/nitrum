FROM amazonlinux:2023

ARG GIT_SHA=unknown
ARG VERSION=dev
LABEL org.opencontainers.image.revision="${GIT_SHA}" \
      org.opencontainers.image.version="${VERSION}" \
      io.nitrum.git.sha="${GIT_SHA}"

RUN dnf install -y aws-nitro-enclaves-cli aws-nitro-enclaves-cli-devel \
    && dnf clean all

ENTRYPOINT ["nitro-cli"]
