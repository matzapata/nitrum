
<!-- host -> gvproxy -->

<!-- cli to work with it all -->
<!-- control plane kick it all off -->
<!-- TODO: pull   -->
<!-- TODO: dev mode -->
<!-- TODO: add prometheus -->

Test in aws:
- cli deployment management
- letsencrypt cert with renewal
- kms
- dynamo state with locks


TODO: use this to add the enclave file to the control-plane, then it's simply running that
```
docker build -t my-new-image -f- . <<'EOF'
FROM my-base-image
COPY extra_script.py /app/
EOF
```

Ingress
Egress
Egress whitelist
Reproducible builds
TLS certificate
TLS certificate with acme
TODO: TLS certificate sync
TODO: properly wait for system to be up
TODO: 


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


Build enclave with docker

```
docker run --rm -v /var/run/docker.sock:/var/run/docker.sock \
  -v $(pwd):/output \
  aws-nitro-enclaves-cli:latest \
  build-enclave \
  --docker-uri matzapata/data-plane:latest \
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