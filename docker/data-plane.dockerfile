################################################################################
# builder
################################################################################

FROM rust:1.88-slim AS builder

WORKDIR /build

ARG FEATURES=""

RUN apt-get update && apt-get install -y musl-tools \
    && rustup target add x86_64-unknown-linux-musl

# Copy Cargo.toml files.
COPY Cargo.toml Cargo.toml
COPY crates/control-plane/Cargo.toml crates/control-plane/Cargo.toml
COPY crates/shared/Cargo.toml crates/shared/Cargo.toml
COPY crates/data-plane/Cargo.toml crates/data-plane/Cargo.toml
COPY crates/cli/Cargo.toml crates/cli/Cargo.toml

# Stub sources so workspace members parse (control-plane depends on shared; cli/data-plane are workspace members).
RUN mkdir -p crates/cli/src crates/control-plane/src crates/data-plane/src \
&& echo 'fn main() {}' > crates/cli/src/main.rs \
&& echo 'fn main() {}' > crates/data-plane/src/main.rs \
&& echo 'fn main() {}' > crates/control-plane/src/main.rs

# Copy sources.
COPY crates/shared/src crates/shared/src
COPY crates/data-plane/src crates/data-plane/src

RUN if [ -n "$FEATURES" ]; then \
        cargo build --release -p data-plane --target x86_64-unknown-linux-musl --features "$FEATURES"; \
    else \
        cargo build --release -p data-plane --target x86_64-unknown-linux-musl; \
    fi

################################################################################
# runtime
################################################################################

FROM alpine:3.20 AS runtime

RUN apk update && apk upgrade
RUN apk --no-cache add curl ca-certificates iproute2

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/data-plane /app/data-plane
RUN chmod +x /app/data-plane

EXPOSE 443
EXPOSE 9090

CMD ["/app/data-plane"]
