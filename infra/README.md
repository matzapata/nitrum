# Welcome to your CDK TypeScript project

This is a blank project for CDK development with TypeScript.

The `cdk.json` file tells the CDK Toolkit how to execute your app.

## Useful commands

* `npm run build`   compile typescript to js
* `npm run watch`   watch for changes and compile
* `npm run test`    perform the jest unit tests
* `npx cdk deploy`  deploy this stack to your default AWS account/region
* `npx cdk diff`    compare deployed stack with current state
* `npx cdk synth`   emits the synthesized CloudFormation template


# Deployment

```bash
export DEPLOYMENT=dev
export CDK_DEPLOY_REGION=sa-east-1
export CDK_DEPLOY_ACCOUNT=$(aws sts get-caller-identity | jq -r '.Account')

cdk deploy NitrumStack -O out.json --require-approval never
```


# Troubleshooting

This document covers common issues and how to debug the nitrum deployment 

---

## Getting cloudwatch logs

```bash
aws logs tail /nitrum/${CDK_PREFIX}/enclave --follow --region $CDK_DEPLOY_REGION --since 1h
```

---

## Debugging the enclave host (EC2)

To inspect the EC2 instance that runs the Nitro Enclave:

```bash
export INSTANCE_ID=$(./scripts/get_asg_instances.sh "$(jq -r '.NitrumStack.ASGGroupName' out.json)")
aws ssm start-session --target "$INSTANCE_ID" --region "$CDK_DEPLOY_REGION"
```

Ensure SSM Session Manager is available (instance role with `AmazonSSMManagedInstanceCore` and SSM agent running). If you use a different output key for the ASG name, replace `NitrumStack.ASGGroupName` accordingly.

**Check host services:**

```bash
sudo systemctl status enclave-watchdog.service
```

All should be active. Inspect logs with `journalctl -u <unit> -f` if needed.

---

## Enclave not running

**Symptom:** Enclave endpoints (e.g. attestation, DEK recover) are unreachable or return connection errors; or you expect an enclave to be up but it is not.

**Check whether any enclave is running:**

```bash
nitro-cli describe-enclaves
```

If the output is `[]`, no enclave is running.

**Restart the enclave manually (for debugging):**

1. Stop the watchdog so it does not restart the enclave while you debug:
   ```bash
   sudo systemctl stop enclave-watchdog.service
   sudo nitro-cli terminate-enclave --all
   ```
2. Start the enclave with console attached (paths may differ; adjust if your playbook uses different locations):
   ```bash
   sudo nitro-cli run-enclave --cpu-count 2 --memory 4320 \
     --eif-path "/home/ec2-user/app/server/enclave.eif" \
     --enclave-cid 16 --enclave-name app --attach-console
   ```
3. Watch console output for errors (certificate fetch, nitriding, app startup). Fix any misconfiguration (e.g. domain, Auth0, RDS, KMS) 

---
