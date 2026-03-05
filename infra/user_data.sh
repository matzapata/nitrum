Content-Type: multipart/mixed; boundary="//"
MIME-Version: 1.0

--//
Content-Type: text/cloud-config; charset="us-ascii"
MIME-Version: 1.0
Content-Transfer-Encoding: 7bit
Content-Disposition: attachment; filename="cloud-config.txt"

#cloud-config
bootcmd:
  - [ dnf, install, aws-nitro-enclaves-cli, aws-nitro-enclaves-cli-devel, htop, git, jq, -y ]

--//
Content-Type: text/x-shellscript; charset="us-ascii"
MIME-Version: 1.0
Content-Transfer-Encoding: 7bit
Content-Disposition: attachment; filename="userdata.txt"

#!/bin/bash

exec > >(tee /var/log/user-data.log | logger -t user-data -s 2>/dev/console) 2>&1

set -x
set +e

usermod -aG docker ec2-user
usermod -aG ne ec2-user

# ── Nitro enclaves allocator ──────────────────────────────────────────────────
ALLOCATOR_YAML=/etc/nitro_enclaves/allocator.yaml
sed -r "s/^(\s*memory_mib\s*:\s*).*/\16144/" -i "$ALLOCATOR_YAML"
sed -r "s/^(\s*cpu_count\s*:\s*).*/\12/" -i "$ALLOCATOR_YAML"

# ── vsock-proxy allowlist ─────────────────────────────────────────────────────
# ${__REGION__} is substituted by CloudFormation Fn::Sub before bash runs.
cat > /etc/nitro_enclaves/vsock-proxy.yaml <<'VSOCK_EOF'
allowlist:
- {address: kms.${__REGION__}.amazonaws.com, port: 443}
- {address: kms-fips.${__REGION__}.amazonaws.com, port: 443}
- {address: 169.254.169.254, port: 80}
VSOCK_EOF

systemctl enable --now docker
systemctl enable --now nitro-enclaves-allocator.service
systemctl enable --now nitro-enclaves-vsock-proxy.service

# ── Pull images and build enclave EIF ────────────────────────────────────────
mkdir -p /home/ec2-user/app/server

# Image URIs are substituted by Fn::Sub; the single-quoted delimiter keeps
# bash from expanding any other $ inside this heredoc.
cat > /home/ec2-user/app/server/build.sh <<'BUILD_EOF'
#!/bin/bash
set -x
set -e

ACCOUNT_ID=$(aws sts get-caller-identity | jq -r '.Account')
TOKEN=$(curl -s -X PUT "http://169.254.169.254/latest/api/token" -H "X-aws-ec2-metadata-token-ttl-seconds: 21600")
REGION=$(curl -s -H "X-aws-ec2-metadata-token: $TOKEN" http://169.254.169.254/latest/meta-data/placement/region)

aws ecr get-login-password --region $REGION \
  | docker login --username AWS --password-stdin \
    $ACCOUNT_ID.dkr.ecr.$REGION.amazonaws.com

docker pull matzapata/nitrum-control-plane:latest
docker tag matzapata/nitrum-control-plane:latest nitrum/control-plane:latest

docker pull ${__ENCLAVE_IMAGE_URI__}
nitro-cli build-enclave \
  --docker-uri ${__ENCLAVE_IMAGE_URI__} \
  --output-file /home/ec2-user/app/server/enclave.eif

BUILD_EOF

chmod +x /home/ec2-user/app/server/build.sh
chown -R ec2-user:ec2-user /home/ec2-user/app

sudo -H -u ec2-user bash /home/ec2-user/app/server/build.sh

# ── Watchdog script ───────────────────────────────────────────────────────────
cat > /home/ec2-user/app/watchdog.py <<'WATCHDOG_EOF'
#!/usr/bin/env python3
"""Enclave watchdog: polls nitro-cli every CHECK_INTERVAL seconds and restarts
the enclave systemd unit whenever the enclave is no longer in RUNNING state."""

import json
import logging
import subprocess
import time

logging.basicConfig(
    level=logging.INFO,
    format="%(asctime)s %(levelname)s %(message)s",
)
logger = logging.getLogger(__name__)

CHECK_INTERVAL_SECONDS = 30


def describe_enclaves() -> list:
    try:
        result = subprocess.run(
            ["nitro-cli", "describe-enclaves"],
            capture_output=True,
            text=True,
            check=True,
        )
        return json.loads(result.stdout)
    except Exception as exc:
        logger.error("describe-enclaves failed: %s", exc)
        return []


def is_enclave_running() -> bool:
    enclaves = describe_enclaves()
    return any(e.get("State") == "RUNNING" for e in enclaves)


def restart_enclave() -> None:
    logger.info("Restarting enclave.service via systemctl...")
    subprocess.run(["systemctl", "restart", "enclave.service"], check=True)
    logger.info("enclave.service restarted.")


def main() -> None:
    logger.info("Watchdog started (interval=%ds).", CHECK_INTERVAL_SECONDS)
    while True:
        try:
            if not is_enclave_running():
                logger.warning("Enclave not running - triggering restart.")
                restart_enclave()
            else:
                logger.debug("Enclave is running.")
        except Exception as exc:
            logger.error("Watchdog iteration failed: %s", exc)
        time.sleep(CHECK_INTERVAL_SECONDS)


if __name__ == "__main__":
    main()
WATCHDOG_EOF

chmod +x /home/ec2-user/app/watchdog.py

# ── Systemd units ─────────────────────────────────────────────────────────────
cat > /etc/systemd/system/control-plane.service <<'CTRL_EOF'
[Unit]
Description=Nitrum Control Plane
After=docker.service nitro-enclaves-allocator.service
Requires=docker.service nitro-enclaves-allocator.service

[Service]
Type=simple
Restart=always
RestartSec=10
ExecStartPre=-/usr/bin/docker rm -f nitrum-control-plane
ExecStart=/usr/bin/docker run --rm \
    --name nitrum-control-plane \
    --device /dev/vsock \
    -p 443:443 \
    -p 8181:8181 \
    -e RUST_LOG=info \
    -e INGRESS_PORT=443 \
    nitrum/control-plane:latest
ExecStop=/usr/bin/docker stop nitrum-control-plane

[Install]
WantedBy=multi-user.target
CTRL_EOF

cat > /etc/systemd/system/enclave.service <<'ENCLAVE_EOF'
[Unit]
Description=Nitrum Enclave
After=nitro-enclaves-allocator.service control-plane.service
Requires=nitro-enclaves-allocator.service control-plane.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/usr/bin/nitro-cli run-enclave \
    --cpu-count 2 \
    --memory 6144 \
    --eif-path /home/ec2-user/app/server/enclave.eif \
    --enclave-cid 16
ExecStop=/bin/bash -c '\
  ENCLAVE_ID=$(nitro-cli describe-enclaves | jq -r ".[0].EnclaveID // empty"); \
  [ -n "$ENCLAVE_ID" ] && nitro-cli terminate-enclave --enclave-id "$ENCLAVE_ID" || true'
Restart=on-failure
RestartSec=15

[Install]
WantedBy=multi-user.target
ENCLAVE_EOF

cat > /etc/systemd/system/enclave-watchdog.service <<'WATCHDOG_SVC_EOF'
[Unit]
Description=Nitrum Enclave Watchdog
After=enclave.service
Wants=enclave.service

[Service]
Type=simple
ExecStart=/usr/bin/python3 /home/ec2-user/app/watchdog.py
Restart=always
RestartSec=30

[Install]
WantedBy=multi-user.target
WATCHDOG_SVC_EOF

systemctl daemon-reload
systemctl enable --now control-plane.service
systemctl enable --now enclave.service
systemctl enable --now enclave-watchdog.service

--//--
