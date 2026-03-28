FROM amazonlinux:2023

RUN dnf install -y aws-nitro-enclaves-cli aws-nitro-enclaves-cli-devel \
    && dnf clean all

ENTRYPOINT ["nitro-cli"]
