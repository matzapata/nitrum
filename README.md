
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
TODO: env vars support (later, inline code enc with kms)
TODO: make sure cert is stored encrypted


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

On **EC2**, the data-plane loads region, instance id, and IAM credentials from **IMDS**, and the DynamoDB table name plus KMS key ID from **SSM** (`/nitrum/dynamodb_table`, `/nitrum/kms_key_id` by default); the CDK stack creates those parameters and grants `ssm:GetParameters`. For **local** runs use `docker/compose/enclave-dev.yml`: **LocalStack** (SSM + DynamoDB) plus **[Amazon EC2 Metadata Mock](https://github.com/aws/amazon-ec2-metadata-mock)** (`public.ecr.aws/aws-ec2/amazon-ec2-metadata-mock:v1.13.0`, config inlined in the compose file as `configs.aemm-config`). Point the app at them with `NITRUM_IMDS_BASE_URL` (default mock port **1338**), `NITRUM_SSM_ENDPOINT_URL`, and `NITRUM_DYNAMODB_ENDPOINT_URL`. SSM parameter names can be overridden only via env vars read by `SsmParameters` (`crates/data-plane/src/utils/ssm.rs`), not `config.rs`.

---

## Running locally with Docker Compose

Full enclave-style stack (IMDS mock + LocalStack SSM/DynamoDB + Pebble + enclave image) lives in **`docker/compose/enclave-dev.yml`**. A smaller reference stack is under **`samples/hello/docker-compose.yml`** (IMDS mock + LocalStack + init).

### 1. Build the data-plane dev image

From the repo root:

```bash
just build-data-plane dev
```

This produces `matzapata/nitrum-data-plane:dev` (no enclave feature; suitable for local runs).

### 2. Start the dev stack

From the repo root (set `ENCLAVE_IMAGE` to your built image):

```bash
export ENCLAVE_IMAGE=matzapata/nitrum-hello:dev   # or your image
docker compose -f docker/compose/enclave-dev.yml up
```

This starts **imds-mock** (AEMM on **1338**), **LocalStack** (SSM + DynamoDB on **4566**), **localstack-init** (table `nitrum-dev` + `/nitrum/*` SSM params), **Pebble**, and the **enclave** service with `NITRUM_IMDS_BASE_URL` / `NITRUM_SSM_ENDPOINT_URL` / `NITRUM_DYNAMODB_ENDPOINT_URL` wired to the mocks.

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

Then provide a **local RSA key** so the data-plane can encrypt/decrypt the DEK without AWS KMS. Generate a key and pass it when starting the stack:

```bash
# Generate key (one-time)
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out kms_private.pem

# Run with key (from samples/hello)
NITRUM_DEV_RSA_PRIVATE_KEY="$(cat kms_private.pem)" docker compose up
```

Alternatively, mount the key file and set the env var in `docker-compose.yml` (see the `NITRUM_DEV_RSA_PRIVATE_KEY` comment in the file).

If you **do not** create the table or set `NITRUM_DEV_RSA_PRIVATE_KEY`, the data-plane still runs but uses an **ephemeral DEK** (lost on restart).

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