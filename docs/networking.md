# Nitrum Networking

This document explains how networking works for a Nitrum workload running in AWS Nitro Enclaves, with emphasis on the host-side `gvproxy`, enclave-side `tap0`, and how inbound and outbound traffic move across the enclave boundary.

## Why networking is special in enclaves

A Nitro Enclave has no direct NIC and no direct Internet access. It communicates through constrained channels exposed by the parent EC2 instance. In Nitrum, the control-plane configures those channels and launches `gvproxy` to provide:

- Port forwarding from host ports to enclave services.
- Layer-3 networking for the enclave over a TAP device.
- Metadata proxy support for IMDS (`169.254.169.254`) when enabled.

Inside the enclave, the data-plane sees a regular network interface (commonly `tap0`) and can run HTTP/TLS services like a standard Linux process.

## Main components

| Component | Where it runs | Role |
| --- | --- | --- |
| `gvproxy` | Parent EC2 instance | Bridges host networking, VSOCK transport, TAP networking, and optional IMDS access into the enclave. |
| `tap0` | Inside enclave | Virtual network interface used by the data-plane for TCP/IP traffic. |
| Data-plane ingress | Inside enclave | Terminates TLS and reverse-proxies app traffic to `127.0.0.1:<project.port>`. |
| User app | Inside enclave | Handles business routes on loopback (`127.0.0.1`). |
| IMDS (`169.254.169.254`) | Host/AWS metadata service | Provides temporary IAM role credentials, proxied through host setup. |

## Topology overview

```mermaid
flowchart LR
    client[External Client]
    nlb[Load Balancer or Host Public Endpoint]
    host[Parent EC2 Instance]
    gv[gvproxy]
    vsock[VSOCK transport]
    tap[tap0 in enclave]
    ingress[Data-plane ingress TLS listener]
    app[User app on 127.0.0.1:project.port]
    imds[IMDS 169.254.169.254]
    aws[AWS APIs KMS/DynamoDB/S3/SSM]

    client --> nlb --> host --> gv
    gv --> vsock --> tap --> ingress --> app
    app --> ingress --> tap --> vsock --> gv --> host --> nlb --> client
    ingress --> tap --> vsock --> gv --> imds
    app --> tap --> vsock --> gv --> aws
```

## Inbound request path (client -> enclave)

For standard HTTPS traffic:

1. A client connects to the public endpoint (for example, NLB -> EC2 host).
2. Host networking forwards traffic to `gvproxy`.
3. `gvproxy` transports packets toward the enclave over VSOCK-backed networking.
4. Inside the enclave, packets appear on `tap0`.
5. Data-plane ingress receives the request, performs TLS termination, and routes:
   - `/.well-known/enclave/*` handled by the data-plane.
   - All other paths proxied to `http://127.0.0.1:<project.port>`.

```mermaid
sequenceDiagram
    autonumber
    actor c as Client
    participant h as Host/NLB
    participant g as gvproxy
    participant t as Enclave tap0
    participant i as Data-plane ingress
    participant a as User app

    c->>h: HTTPS request
    h->>g: Forward TCP:443
    g->>t: Deliver via VSOCK/TAP bridge
    t->>i: Packet to ingress listener
    i->>i: TLS termination + route match
    alt well-known route
        i-->>c: Data-plane response
    else app route
        i->>a: HTTP on 127.0.0.1:project.port
        a-->>i: App response
        i-->>c: HTTPS response
    end
```

## Outbound path (enclave -> AWS and Internet services)

When the app or data-plane needs egress (for example KMS, DynamoDB, S3, ACME), traffic leaves via `tap0`, crosses `gvproxy` on the host, then exits through normal host networking.

```mermaid
sequenceDiagram
    autonumber
    participant a as App/Data-plane in enclave
    participant t as tap0
    participant g as gvproxy on host
    participant h as Host network stack
    participant s as External service AWS/ACME

    a->>t: TCP/IP packet (eg HTTPS to service)
    t->>g: Forward over enclave-host transport
    g->>h: Re-inject into host networking
    h->>s: Outbound connection
    s-->>h: Response
    h-->>g: Return packets
    g-->>t: Deliver back into enclave
    t-->>a: Response data
```

## IMDS credentials path

Nitrum commonly enables metadata access through `gvproxy`, allowing enclave software to query `http://169.254.169.254/latest` and obtain IAM role credentials (IMDSv2 flow). The endpoint is reached from inside the enclave through the same `tap0` -> host path.

```mermaid
sequenceDiagram
    autonumber
    participant d as Data-plane/App
    participant t as tap0
    participant g as gvproxy
    participant m as IMDS 169.254.169.254
    participant k as AWS KMS

    d->>m: IMDSv2 token and role credentials request
    Note over d,g: Routed through tap0 + gvproxy
    m-->>d: Temporary AWS credentials
    d->>k: Signed AWS API request with temp creds
    k-->>d: API response
```

## Local loopback vs enclave edge

Inside the enclave there are two important traffic scopes:

- Enclave edge traffic: arrives on ingress listener via `tap0` and is visible to external clients.
- Local loopback traffic: `127.0.0.1` communication between data-plane and your app (and optional internal APIs).

This split lets Nitrum keep the app interface simple (regular localhost HTTP) while keeping TLS and attestation logic at the enclave edge.

## Practical implications

- TLS is terminated inside the enclave, not on the parent host.
- `gvproxy` is a required host-side networking dependency for traffic and metadata bridging.
- If host-side `gvproxy` or port mapping is misconfigured, enclave services may be healthy internally but unreachable externally.
- If metadata access is disabled/misconfigured, in-enclave AWS SDK calls may fail due to missing credentials.
- Networking behavior is deterministic once `tap0`, forwarding rules, and ingress bindings are stable.

## Related docs

- [Architecture](architecture.md) for control-plane/data-plane internals and cryptographic flows.
- [Usage](usage.md) for command-level deployment and runtime configuration details.
