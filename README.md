
Ingress
Egress
Egress whitelist
Reproducible builds
TLS certificate
TLS certificate with acme
TODO: TLS certificate sync

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