################################################################################
# gvproxy builder
################################################################################

FROM golang:1.25 AS gvproxy-builder

WORKDIR /

RUN git clone --depth 1 --branch v0.7.4 https://github.com/containers/gvisor-tap-vsock.git
RUN cd gvisor-tap-vsock && CGO_ENABLED=0 GOARCH=amd64 GOOS=linux go build -ldflags '-extldflags "-static"' -o bin/gvproxy-linux-amd64 ./cmd/gvproxy

################################################################################
# Runtime
################################################################################

FROM alpine:3.20 AS runtime

RUN apk update && apk upgrade
RUN apk --no-cache add curl ca-certificates

COPY --from=gvproxy-builder /gvisor-tap-vsock/bin/gvproxy-linux-amd64 /app/gvproxy
COPY docker/control-plane-entrypoint.sh /app/entrypoint.sh

RUN chmod +x /app/gvproxy /app/entrypoint.sh

EXPOSE 443
EXPOSE 9090

CMD ["/app/entrypoint.sh"]
