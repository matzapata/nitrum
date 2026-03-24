
<!-- host -> gvproxy -->

<!-- cli to work with it all -->
<!-- control plane kick it all off -->
<!-- TODO: pull   -->
<!-- TODO: dev mode -->
<!-- TODO: add prometheus -->
<!-- TODO: just run it all locally and  -->
<!-- TODO: anyhow errors improvements -->

Test in aws:
- cli deployment management, scaling update
- pebble docker compose for sample
- kms
- dynamo state with locks

Ingress
Egress
TODO: Egress whitelist
Reproducible builds. Ok?
TLS certificate
TLS certificate with acme
TODO: TLS certificate sync
TODO: properly wait for system to be up
TODO: add cert to attestation
TODO: env vars support (later, POST request, add authority pub key in image)
TODO: make sure cert is stored encrypted
TODO: reproducible builds
TODO: control plane watchdog style, reset if killed, logs, etc
TODO: cloudformation template instead of cdk


KMS to encrypt decript data (use public key to wrap a sync key and store the sync key in db)

IMDS? 
- KMS
- Cloudwatch -> log group so far is created in cdk but not used by the rust app
- Dynamodb

attestations
JS sdk for attestation verification client side

CLI to get the PCR0
CLI to deploy to aws


sample usage for mcp


Test in aws
- KMS
- attestation
- acme
- vsock

curl -k https://Nitrum-Nitro-JFeawD8Jimon-8a5ef6f56c779882.elb.sa-east-1.amazonaws.com/health


Build enclave with Docker (same flow as `nitrum build`): build the app image first, then run `nitro-cli` in a container. The `--docker-uri` value must be **exactly** the tag you passed to `docker build -t` (use a `matzapata/...` name so a cache miss does not try to pull a private repo from Hub).

On **Apple Silicon**, both images must be **linux/amd64**: the enclave image (Nitro EIF is amd64), and the nitro-cli wrapper image—otherwise linuxkit looks for an arm64 enclave image, finds none, and tries to pull from Docker Hub. Build the CLI image with `docker build --platform linux/amd64 …` once, then:

```
docker build --platform linux/amd64 -f Dockerfile \
  --build-arg DATA_PLANE_IMAGE=matzapata/nitrum-data-plane:latest \
  -t matzapata/nitrum-enclave:latest .

docker run --rm --platform linux/amd64 \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v $(pwd):/output \
  matzapata/nitrum-nitro-cli:latest \
  build-enclave \
  --docker-uri matzapata/nitrum-enclave:latest \
  --output-file /output/enclave.eif
```



```bash
# build for prod
just build-data-plane
just build-control-plane

# build for local running
just build-data-plane dev
just build-control-plane dev

# To rebuild without cache
just build-data-plane dev true
```

On **EC2**, the data-plane loads region, instance id, and IAM credentials from **IMDS**, and the DynamoDB table name plus KMS key ID from **SSM** (`/nitrum/dynamodb_table`, `/nitrum/kms_key_id` by default); the CDK stack creates those parameters and grants `ssm:GetParameters`. **Inside a Nitro enclave**, IMDSv2 uses the same link-local URL as on the host (**`http://169.254.169.254/latest`**) over the **TAP ↔ gvproxy** path; the control-plane must run **[gvisor-tap-vsock](https://github.com/containers/gvisor-tap-vsock) gvproxy** (v0.8.7+) with **`-ec2-metadata-access`** so TCP to the metadata address is forwarded (see [PR #512](https://github.com/containers/gvisor-tap-vsock/pull/512)). That replaces a separate loopback **viproxy** plus parent **vsock-proxy** / **`enclave-imds-proxy`** for IMDS. ACME HTTP-01 still listens on **`0.0.0.0:80`** on the data-plane; set **`NITRUM_IMDS_BASE_URL`** for metadata mocks or unusual layouts. For **local** runs use **`infra/docker/compose.yml`** (the CLI writes the same content to **`.nitrum/docker-compose.yml`** the first time you run **`nitrum dev up`**, **`dev down`**, or **`dev logs`** if that file is missing): **LocalStack** (SSM + DynamoDB) plus **[Amazon EC2 Metadata Mock](https://github.com/aws/amazon-ec2-metadata-mock)** (`public.ecr.aws/aws-ec2/amazon-ec2-metadata-mock:v1.13.0`, config inlined in the compose file as `configs.aemm-config`). Compose sets **`NITRUM_IMDS_BASE_URL`** to the mock (e.g. `http://imds:1338/latest`), plus `NITRUM_SSM_ENDPOINT_URL` and `NITRUM_DYNAMODB_ENDPOINT_URL`. SSM parameter names can be overridden only via env vars read by `SsmParameters` (`crates/data-plane/src/utils/ssm.rs`), not `config.rs`.

---

## Running locally with Docker Compose

Full enclave-style stack (IMDS mock + LocalStack SSM/DynamoDB + Pebble + enclave image) lives in **`infra/docker/compose.yml`**. **`nitrum dev up`** (and other **`nitrum dev`** commands) use **`.nitrum/docker-compose.yml`**: the CLI creates it from the bundled compose file if it is not there yet (same content as **`infra/docker/compose.yml`**).

### 1. Build the data-plane dev image

From the repo root:

```bash
just build-data-plane dev
```

This produces `matzapata/nitrum-data-plane:dev` (no enclave feature; suitable for local runs).

### 2. Start the dev stack

From an initialized project (or repo root with `-f`), set `ENCLAVE_IMAGE` to your built image (same pattern as **`nitrum dev up`**: `nitrum-{name}` from top-level **`name`** in `nitrum.toml`, tag **`dev`**):

```bash
export ENCLAVE_IMAGE=nitrum-nitrum-hello:dev   # example when name = "nitrum-hello" in nitrum.toml
docker compose -f infra/docker/compose.yml up
```

From a Nitrum project root, run **`nitrum dev up`** (the CLI ensures **`.nitrum/docker-compose.yml`** exists before starting compose).

This starts **imds-mock** (AEMM on **1338**), **LocalStack** (SSM + DynamoDB + KMS on **4566**), **localstack-init** (table `nitrum-dev`, a symmetric KMS CMK, `/nitrum/*` SSM params), **Pebble**, and the **enclave** service with mock endpoints including `NITRUM_KMS_ENDPOINT_URL` wired to LocalStack.

The data-plane uses **self-signed TLS** by default. All `curl` examples below use `-k` to skip certificate verification.

### 3. DynamoDB table and SSM (handled by compose)

The compose file creates the DynamoDB table and SSM parameters. To recreate manually against LocalStack:

```bash
aws dynamodb create-table \
  --endpoint-url http://localhost:4566 \
  --region us-east-1 \
  --table-name nitrum-dev \
  --attribute-definitions AttributeName=pk,AttributeType=S \
  --key-schema AttributeName=pk,KeyType=HASH \
  --billing-mode PAY_PER_REQUEST \
  --no-cli-pager
```

The DEK is wrapped with **KMS** (`GenerateDataKeyWithoutPlaintext`); the compose init creates a symmetric CMK and stores its id in SSM as `/nitrum/kms_key_id`. The enclave service sets `NITRUM_KMS_ENDPOINT_URL=http://localstack:4566` so the data-plane talks to LocalStack KMS.

To recreate the key parameter manually:

```bash
KEY_ID=$(aws kms create-key \
  --endpoint-url http://localhost:4566 \
  --region us-east-1 \
  --description nitrum-dev-symmetric-data-key \
  --query KeyMetadata.KeyId --output text)

aws ssm put-parameter \
  --endpoint-url http://localhost:4566 \
  --region us-east-1 \
  --name /nitrum/kms_key_id \
  --value "$KEY_ID" \
  --type String \
  --overwrite
```

If the table or KMS/SSM setup is missing, the data-plane may still run with an **ephemeral DEK** (lost on restart), depending on code paths.

### 4. Test endpoints

| What | Command |
|------|--------|
| Enclave well-known (no user app) | `curl -sk https://localhost/.well-known/enclave/status` |
| Enclave attestation | `curl -sk https://localhost/.well-known/enclave/attestation` |
| User app (via TLS) | `curl -sk https://localhost/health` |
| Internal API (no TLS) | `curl http://localhost:3000/attestation -X POST -H "Content-Type: application/json" -d '{}'` |

Or run the sample’s e2e script (from `samples/hello`):

```bash
just e2e
```

### Optional: Pebble (ACME)

To test ACME certificate issuance locally, uncomment the **pebble** service and the `PEBBLE_*` env vars in `samples/hello/docker-compose.yml`, then switch the data-plane to use `tls::acme()` instead of `tls::self_signed()` in `main.rs`. Mount the Pebble minica cert (e.g. from `tests/certs/pebble.minica.pem`) so the data-plane trusts Pebble’s CA.