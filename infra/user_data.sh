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

--//--
