#!/usr/bin/env bash
set -e
set +x

output="${1}"

# get values
asg_name=$(jq -r '.NitrumStack.ASGGroupName' "${output}")
instance_id=$(./scripts/get_asg_instances.sh "${asg_name}" | head -n 1)
pcr_0=$(./scripts/get_pcr0.sh "${instance_id}") # pcr_0 for debug mode: 000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000
ec2_role_arn=$(jq -r '.NitrumStack.EC2InstanceRoleARN' "${output}")
account_id=$(aws sts get-caller-identity | jq -r '.Account')

# render policy
jq --arg pcr_0 "$pcr_0" \
   --arg ec2_role_arn "$ec2_role_arn" \
   --arg lambda_execution_arn "$lambda_execution_arn" \
   --arg account_id "arn:aws:iam::${account_id}:root" \
   '.Statement[0].Condition.StringEqualsIgnoreCase."kms:RecipientAttestation:ImageSha384"=$pcr_0 |
    .Statement[0].Principal.AWS=$ec2_role_arn |
    .Statement[1].Principal.AWS=$account_id' \
   ./scripts/kms_key_policy_template.json