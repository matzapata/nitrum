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
export APP_DIRECTORY=/Users/matzapata/git/enclaves-poc/nitrum/samples/hello

cdk deploy NitrumStack -O out.json --require-approval never
```

## Run End-To-End (gvproxy + enclave)

This flow builds the app image, deploys infra, ensures `gvproxy` is running on the host VSOCK port, and validates ingress/egress.

### 1) Build the sample image (no manual data-plane prebuild)

From the repository root:

```bash
docker build --platform linux/amd64 --build-arg FEATURES=enclave -f samples/hello/Dockerfile -t matzapata/nitrum-hello:latest .
docker push matzapata/nitrum-hello:latest
```

The sample Dockerfile builds `data-plane` from source in the same build. The control-plane binary starts gvproxy on launch (terminating any existing instance first) and stops it on exit.

### 2) Deploy infra

```bash
export DEPLOYMENT=dev
export CDK_DEPLOY_REGION=sa-east-1
export CDK_DEPLOY_ACCOUNT=$(aws sts get-caller-identity | jq -r '.Account')
export APP_DIRECTORY=/Users/matzapata/git/enclaves-poc/nitrum/samples/hello

cdk deploy NitrumStack -O out.json --require-approval never
```

### 3) Ensure gvproxy is running on the EC2 host

Your host-side service should start `gvproxy` with VSOCK listening on `:1024` and configure forwarding rules for `443` and `9090`.

Expected command shape:

```bash
./gvproxy -listen vsock://:1024 -listen unix:///tmp/network.sock
```

And exposes:
- host `:443` -> enclave `192.168.127.2:443`
- host `:9090` -> enclave `192.168.127.2:9090`

The enclave-side config must match that port:

```toml
[network]
host_proxy_port = 1024
```

### 4) Verify on host

```bash
export INSTANCE_ID=$(./scripts/get_asg_instances.sh "$(jq -r '.NitrumStack.ASGGroupName' out.json)")
aws ssm start-session --target "$INSTANCE_ID" --region "$CDK_DEPLOY_REGION"
```

Inside the instance:

```bash
sudo systemctl status gvproxy.service
sudo systemctl status control-plane.service
sudo systemctl status enclave.service
nitro-cli describe-enclaves
curl -sk https://localhost/health
curl -sk https://localhost/egress | head -c 200 && echo
```

---


# Troubleshooting

This document covers common issues and how to debug the nitrum deployment 

---

### `Missing required env: CDK_DEPLOY_ACCOUNT` (or `CDK_DEPLOY_REGION`)

`cdk deploy` reads account and region from the environment. Export them (and the other variables below) **before** deploying—the same way as in **Deployment** at the top of this file:

```bash
export CDK_DEPLOY_REGION=sa-east-1
export CDK_DEPLOY_ACCOUNT=$(aws sts get-caller-identity | jq -r '.Account')
```

Replace `sa-east-1` with your region. You need working AWS CLI credentials so `aws sts get-caller-identity` returns your account. Also set `DEPLOYMENT` and `EIF_PATH` (see the full **Deployment** snippet).

---

```bash
export DEPLOYMENT=dev
export CDK_DEPLOY_REGION=sa-east-1
export CDK_DEPLOY_ACCOUNT=$(aws sts get-caller-identity | jq -r '.Account')
export APP_DIRECTORY=/Users/matzapata/git/enclaves-poc/nitrum/samples/hello
aws ssm start-session --target $(./scripts/get_asg_instances.sh "$(jq -r '.NitrumStack.ASGGroupName' out.json)") --region "$CDK_DEPLOY_REGION"

sudo docker pull matzapata/nitrum-control-plane:latest && sudo docker pull matzapata/nitrum-hello:latest
sudo mkdir -p /home/ec2-user/app/server && sudo chown ec2-user:ec2-user /home/ec2-user/app/server
sudo nitro-cli build-enclave \
  --docker-uri matzapata/demo-iptables:latest \
  --output-file /home/ec2-user/app/server/enclave.eif

sudo docker image inspect matzapata/nitrum-hello:latest --format='{{index .RepoDigests 0}}'


# rebuild
sudo -H -u ec2-user bash /home/ec2-user/app/server/build.sh

# Get services statuses
sudo systemctl status control-plane.service
sudo systemctl status enclave.service

# Get enclave
nitro-cli describe-enclaves

sudo cat /var/log/user-data.log
sudo journalctl -u control-plane.service -n 200
sudo journalctl -u enclave.service -n 200

# terminate it all
sudo systemctl stop control-plane.service
sudo systemctl stop enclave.service
sudo nitro-cli terminate-enclave --all
sudo docker rm -f nitrum-control-plane

# Start with console attached
sudo nitro-cli run-enclave --cpu-count 2 --memory 4320 --eif-path "/home/ec2-user/app/server/enclave.eif" --enclave-cid 16 --enclave-name app --attach-console

aws logs tail /nitrum/${CDK_PREFIX}/enclave --follow --region $CDK_DEPLOY_REGION --since 1h
```

<!-- TODO: better organize these docs -->

(
sudo docker run --rm --name nitrum-control-plane \
  --privileged \
  --security-opt seccomp=unconfined \
  -p 443:443 \
  -e RUST_LOG=info \
  -e INGRESS_PORT=443 \
  nitrum/control-plane:latest \
| sed 's/^/[control-plane] /'
) &
(
sudo nitro-cli run-enclave \
  --cpu-count 2 \
  --memory 4320 \
  --eif-path "/home/ec2-user/app/server/enclave.eif" \
  --enclave-cid 16 \
  --enclave-name app \
  --attach-console \
| sed 's/^/[enclave] /'
) &
(
while true; do
  curl -s https://localhost:443 | sed 's/^/[curl] /'
  sleep 3
done
) &
wait

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
sudo systemctl status control-plane.service
sudo systemctl status enclave.service
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
   sudo systemctl stop control-plane.service
   sudo systemctl stop enclave.service
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




```
sudo bash -c 'cat << "EOF" > run.sh
#!/usr/bin/env bash

sudo docker pull matzapata/nitrum-control-plane:latest && sudo docker pull matzapata/nitrum-hello:latest
sudo mkdir -p /home/ec2-user/app/server && sudo chown ec2-user:ec2-user /home/ec2-user/app/server
sudo nitro-cli build-enclave \
  --docker-uri matzapata/nitrum-hello:latest \
  --output-file /home/ec2-user/app/server/enclave.eif

cleanup() {
  echo "Stopping all processes..."
  kill "$cp_pid" "$enclave_pid" "$curl_pid" 2>/dev/null
  wait
  exit
}

trap cleanup INT TERM

(
sudo docker run --rm --name nitrum-control-plane \
  --privileged \
  --security-opt seccomp=unconfined \
  -p 443:443 \
  -e RUST_LOG=info \
  -e INGRESS_PORT=443 \
  matzapata/nitrum-control-plane:latest \
| sed '\''s/^/[control-plane] /'\''
) &
cp_pid=$!

(
sudo nitro-cli run-enclave \
  --cpu-count 2 \
  --memory 4320 \
  --eif-path "/home/ec2-user/app/server/enclave.eif" \
  --enclave-cid 16 \
  --enclave-name app \
  --attach-console \
| sed '\''s/^/[enclave] /'\''
) &
enclave_pid=$!

(
while true; do
  curl -s https://localhost:443/health | sed '\''s/^/[curl] /'\''
  sleep 3
done
) &
curl_pid=$!

wait
EOF' && sudo chmod +x ./run.sh && ./run.sh


```





```bash
export DEPLOYMENT=dev
export CDK_DEPLOY_REGION=sa-east-1
export CDK_DEPLOY_ACCOUNT=$(aws sts get-caller-identity | jq -r '.Account')
export APP_DIRECTORY=/Users/matzapata/git/enclaves-poc/nitrum/samples/hello
aws ssm start-session --target $(./scripts/get_asg_instances.sh "$(jq -r '.NitrumStack.ASGGroupName' out.json)") --region "$CDK_DEPLOY_REGION"

sudo docker pull matzapata/nitrum-control-plane:latest && sudo docker pull matzapata/nitrum-hello:latest
sudo mkdir -p /home/ec2-user/app/server && sudo chown ec2-user:ec2-user /home/ec2-user/app/server
sudo nitro-cli build-enclave \
  --docker-uri matzapata/nitrum-hello:latest \
  --output-file /home/ec2-user/app/server/enclave.eif

sudo docker image inspect matzapata/nitrum-hello:latest --format='{{index .RepoDigests 0}}'


# rebuild
sudo -H -u ec2-user bash /home/ec2-user/app/server/build.sh

# Get services statuses
sudo systemctl status control-plane.service
sudo systemctl status enclave.service

# Get enclave
nitro-cli describe-enclaves

sudo cat /var/log/user-data.log
sudo journalctl -u control-plane.service -n 200
sudo journalctl -u enclave.service -n 200

# terminate it all
sudo systemctl stop control-plane.service
sudo systemctl stop enclave.service
sudo nitro-cli terminate-enclave --all
sudo docker rm -f nitrum-control-plane

# Start with console attached
sudo nitro-cli run-enclave --cpu-count 2 --memory 4320 --eif-path "/home/ec2-user/app/server/enclave.eif" --enclave-cid 16 --enclave-name app --attach-console

aws logs tail /nitrum/${CDK_PREFIX}/enclave --follow --region $CDK_DEPLOY_REGION --since 1h
```

<!-- TODO: better organize these docs -->

(
sudo docker run --rm --name nitrum-control-plane \
  --privileged \
  --security-opt seccomp=unconfined \
  -p 443:443 \
  -e RUST_LOG=info \
  -e INGRESS_PORT=443 \
  nitrum/control-plane:latest \
| sed 's/^/[control-plane] /'
) &
(
sudo nitro-cli run-enclave \
  --cpu-count 2 \
  --memory 4320 \
  --eif-path "/home/ec2-user/app/server/enclave.eif" \
  --enclave-cid 16 \
  --enclave-name app \
  --attach-console \
| sed 's/^/[enclave] /'
) &
(
while true; do
  curl -s https://localhost:443 | sed 's/^/[curl] /'
  sleep 3
done
) &
wait

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
sudo systemctl status control-plane.service
sudo systemctl status enclave.service
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
   sudo systemctl stop control-plane.service
   sudo systemctl stop enclave.service
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


```
sudo bash -c 'cat << "EOF" > run.sh
#!/usr/bin/env bash
set -euo pipefail

sudo docker pull matzapata/gvproxy:latest && sudo docker pull matzapata/nitrum-hello:latest
sudo mkdir -p /home/ec2-user/app/server && sudo chown ec2-user:ec2-user /home/ec2-user/app/server && sudo nitro-cli build-enclave --docker-uri matzapata/nitrum-hello:latest --output-file /home/ec2-user/app/server/enclave.eif

cleanup() {
  echo "Stopping all processes..."
  kill "${gvproxy_logs_pid:-}" "${enclave_pid:-}" "${curl_pid:-}" 2>/dev/null || true
  sudo docker rm -f gvproxy >/dev/null 2>&1 || true
  sudo nitro-cli terminate-enclave --all >/dev/null 2>&1 || true
  wait || true
  exit
}

trap cleanup INT TERM

sudo docker rm -f gvproxy >/dev/null 2>&1 || true
sudo docker run -d --name gvproxy \
  --privileged \
  --security-opt seccomp=unconfined \
  -p 443:443 \
  -p 9090:9090 \
  matzapata/gvproxy:latest >/dev/null

# Stream gvproxy logs in the background.
(
sudo docker logs -f gvproxy | sed '\''s/^/[gvproxy] /'\''
) &
gvproxy_logs_pid=$!

(
sudo nitro-cli run-enclave \
  --cpu-count 2 \
  --memory 4320 \
  --eif-path "/home/ec2-user/app/server/enclave.eif" \
  --enclave-cid 16 \
  --enclave-name app \
  --attach-console \
| sed '\''s/^/[enclave] /'\''
) &
enclave_pid=$!

# Wait briefly for gvproxy + enclave startup before probing.
sleep 5

(
while true; do
  echo "[curl] GET https://localhost:443/health"
  if ! curl -sk --max-time 5 https://localhost:443/health | sed '\''s/^/[curl] /'\''; then
    echo "[curl] request failed"
  fi
  sleep 3
done
) &
curl_pid=$!

wait
EOF' && sudo chmod +x ./run.sh && ./run.sh


```

export DEPLOYMENT=dev
export CDK_DEPLOY_REGION=sa-east-1
export CDK_DEPLOY_ACCOUNT=$(aws sts get-caller-identity | jq -r '.Account')
export APP_DIRECTORY=/Users/matzapata/git/enclaves-poc/nitrum/samples/hello
aws ssm start-session --target $(./scripts/get_asg_instances.sh "$(jq -r '.NitrumStack.ASGGroupName' out.json)") --region "$CDK_DEPLOY_REGION"


sudo docker pull matzapata/nitrum-control-plane:latest && sudo docker pull matzapata/nitrum-hello:latest

sudo nitro-cli build-enclave --docker-uri matzapata/nitrum-hello:latest --output-file /usr/bin/enclave.eif


sudo docker run -d --name control-plane \
  --privileged \
  --security-opt seccomp=unconfined \
  -p 443:443 \
  -p 9090:9090 \
  -v /usr/bin/enclave.eif:/app/enclave.eif \
  matzapata/nitrum-control-plane:latest /app/control-plane --debug-mode

sudo docker logs control-plane

curl -k https://Nitrum-Nitro-x2FdsBl5rVBA-9de4f60889c4b629.elb.sa-east-1.amazonaws.com/health

curl -k https://localhost/health

sudo docker run -d --name gvproxy \
  --privileged \
  --security-opt seccomp=unconfined \
  -p 443:443 \
  -p 9090:9090 \
  matzapata/gvproxy:latest 


sudo docker run \
  -v $(pwd):/app \
  --privileged \
  --security-opt seccomp=unconfined \
  matzapata/nitrum-control-plane:latest \
  sh

sudo nitro-cli run-enclave \
  --cpu-count 2 \
  --memory 4320 \
  --eif-path "/usr/bin/enclave.eif" \
  --enclave-cid 16 \
  --enclave-name app \
  --attach-console 


  aws ssm start-session --target $(.nitrum/infra/scripts/get_asg_instances.sh "$(jq -r '.NitrumStack.ASGGroupName' out.json)") --region "$CDK_DEPLOY_REGION"
