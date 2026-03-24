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

# Nitro enclaves allocator
ALLOCATOR_YAML=/etc/nitro_enclaves/allocator.yaml
sed -r "s/^(\s*memory_mib\s*:\s*).*/\16144/" -i "$ALLOCATOR_YAML"
sed -r "s/^(\s*cpu_count\s*:\s*).*/\12/" -i "$ALLOCATOR_YAML"

# Enable services
systemctl enable --now docker
systemctl enable --now nitro-enclaves-allocator.service
systemctl enable --now nitro-enclaves-vsock-proxy.service

# ── Wait for services before pulling containers and downloading EIF ───────────
sleep 5

# ── Pull control-plane image and download enclave EIF from S3 ────────────────
CONTROL_PLANE_IMAGE="matzapata/nitrum-control-plane:latest"
EIF_ASSET_HASH="${__EIF_ASSET_HASH__}"

docker pull "$CONTROL_PLANE_IMAGE"
aws s3 cp "s3://${__EIF_S3_BUCKET__}/${__EIF_S3_KEY__}" /usr/bin/enclave.eif --region "${__REGION__}"

# TODO: remove debug mode
# ── Systemd unit for control-plane (gvproxy + enclave) ───────────────────────
cat > /etc/systemd/system/control-plane.service <<'UNIT_EOF'
[Unit]
Description=Nitrum control-plane (gvproxy + enclave)
After=docker.service nitro-enclaves-allocator.service
Requires=docker.service

[Service]
Type=simple
Restart=always
RestartSec=10
ExecStartPre=-/usr/bin/docker rm -f control-plane
ExecStart=/usr/bin/docker run --rm --name control-plane \
  --privileged \
  --security-opt seccomp=unconfined \
  -p 443:443 \
  -p 9090:9090 \
  -v /usr/bin/enclave.eif:/app/enclave.eif \
  matzapata/nitrum-control-plane:latest /app/control-plane --debug-mode
ExecStop=/usr/bin/docker stop -t 10 control-plane
TimeoutStopSec=30

[Install]
WantedBy=multi-user.target
UNIT_EOF

systemctl daemon-reload
systemctl enable --now control-plane.service

--//--
