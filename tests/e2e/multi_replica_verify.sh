#!/usr/bin/env bash
# Post-deploy verification for horizontal scaling (does not deploy).
#
# Prerequisites: stack already deployed with desired_replicas >= 2, aws CLI, curl.
#
# Usage:
#   ./tests/e2e/multi_replica_verify.sh --stack-name nitrum-myproject --region us-east-1
#   ./tests/e2e/multi_replica_verify.sh --stack-name nitrum-myproject --region us-east-1 \
#     --expected-replicas 2 --nlb-url https://my-nlb.elb.amazonaws.com
#
# Exit 0 when infrastructure and metric checks pass; attestation identity across
# instances may still require per-host access (SSM / Instance Connect) as printed.

set -euo pipefail

STACK_NAME=""
AWS_REGION="${AWS_REGION:-${AWS_DEFAULT_REGION:-}}"
EXPECTED_REPLICAS=2
NLB_URL=""
PROJECT_NAME=""

usage() {
    sed -n '2,12p' "$0" | sed 's/^# \?//'
    exit 1
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --stack-name) STACK_NAME="$2"; shift 2 ;;
        --region) AWS_REGION="$2"; shift 2 ;;
        --expected-replicas) EXPECTED_REPLICAS="$2"; shift 2 ;;
        --nlb-url) NLB_URL="$2"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "unknown arg: $1" >&2; usage ;;
    esac
done

[[ -n "${STACK_NAME}" && -n "${AWS_REGION}" ]] || usage
command -v aws >/dev/null || { echo "aws CLI required" >&2; exit 1; }

aws_args=(--region "${AWS_REGION}")

stack_output() {
    local key="$1"
    aws cloudformation describe-stacks "${aws_args[@]}" \
        --stack-name "${STACK_NAME}" \
        --query "Stacks[0].Outputs[?OutputKey==\`${key}\`].OutputValue | [0]" \
        --output text
}

PROJECT_NAME="$(stack_output ProjectName)"
ASG_NAME="$(stack_output ASGGroupName)"
if [[ -z "${NLB_URL}" ]]; then
    nlb="$(stack_output NLBDnsName)"
    NLB_URL="https://${nlb}"
fi
NLB_URL="${NLB_URL%/}"

echo "=== multi-replica verify: stack=${STACK_NAME} project=${PROJECT_NAME} replicas=${EXPECTED_REPLICAS} ==="

# ASG in-service count
in_service="$(aws autoscaling describe-auto-scaling-groups "${aws_args[@]}" \
    --auto-scaling-group-names "${ASG_NAME}" \
    --query 'AutoScalingGroups[0].Instances[?LifecycleState==`InService`] | length(@)' \
    --output text)"
desired="$(aws autoscaling describe-auto-scaling-groups "${aws_args[@]}" \
    --auto-scaling-group-names "${ASG_NAME}" \
    --query 'AutoScalingGroups[0].DesiredCapacity' \
    --output text)"

echo "ASG ${ASG_NAME}: desired=${desired} in_service=${in_service}"
if [[ "${desired}" != "${EXPECTED_REPLICAS}" || "${in_service}" -lt "${EXPECTED_REPLICAS}" ]]; then
    echo "FAIL: expected desired and in_service >= ${EXPECTED_REPLICAS}" >&2
    exit 1
fi

# NLB target health (HTTPS target group)
tg_arn="$(aws elbv2 describe-target-groups "${aws_args[@]}" \
    --names "${PROJECT_NAME}-tg" \
    --query 'TargetGroups[0].TargetGroupArn' \
    --output text)"
healthy="$(aws elbv2 describe-target-health "${aws_args[@]}" \
    --target-group-arn "${tg_arn}" \
    --query 'length(TargetHealthDescriptions[?TargetHealth.State==`healthy`])' \
    --output text)"
echo "Target group ${PROJECT_NAME}-tg: healthy=${healthy}"
if [[ "${healthy}" -lt "${EXPECTED_REPLICAS}" ]]; then
    echo "FAIL: expected ${EXPECTED_REPLICAS} healthy targets" >&2
    exit 1
fi

# CloudWatch: distinct InstanceId series with EnclaveRunning near 1
end=$(date -u +%Y-%m-%dT%H:%M:%SZ)
start=$(date -u -v-15M +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d '15 minutes ago' +%Y-%m-%dT%H:%M:%SZ)
metric_json="$(aws cloudwatch get_metric_data "${aws_args[@]}" \
    --metric-data-queries "[{\"Id\":\"e1\",\"Expression\":\"SEARCH('{Nitrum/${PROJECT_NAME},Component,InstanceId} MetricName=\\\"EnclaveRunning\\\"','Average',60)',\"ReturnData\":true}]" \
    --start-time "${start}" \
    --end-time "${end}" \
    --output json)"

running_instances="$(echo "${metric_json}" | python3 -c "
import json, sys
data = json.load(sys.stdin)
ids = set()
for r in data.get('MetricDataResults', []):
    label = r.get('Label') or ''
    vals = [v for v in r.get('Values', []) if v is not None]
    if not vals:
        continue
    if max(vals) >= 0.5:
        # Label often contains InstanceId dimension value
        for part in label.replace(',', ' ').split():
            if part.startswith('i-'):
                ids.add(part)
        # MetricDataResults may use Id only; scan Dimensions in raw if present
for r in data.get('MetricDataResults', []):
    for d in r.get('Dimensions', []) or []:
        if d.get('Name') == 'InstanceId' and (r.get('Values') or []) and max(r['Values']) >= 0.5:
            ids.add(d['Value'])
print(len(ids))
" 2>/dev/null || echo "0")"

echo "CloudWatch EnclaveRunning instances (approx): ${running_instances}"
if [[ "${running_instances}" -lt "${EXPECTED_REPLICAS}" ]]; then
    echo "WARN: could not confirm ${EXPECTED_REPLICAS} InstanceId series via SEARCH; check ${PROJECT_NAME}-ops dashboard manually" >&2
fi

# NLB attestation smoke (same cert on any backend — full per-instance compare needs SSM)
if command -v curl >/dev/null; then
    echo "=== attestation smoke via NLB (${NLB_URL}) ==="
  att_path="/.well-known/enclave/attestation"
  hash1="$(curl -sk "${NLB_URL}${att_path}" | sha256sum | awk '{print $1}')"
  hash2="$(curl -sk "${NLB_URL}${att_path}" | sha256sum | awk '{print $1}')"
  echo "attestation body sha256 (request 1): ${hash1}"
  echo "attestation body sha256 (request 2): ${hash2}"
  if [[ "${hash1}" != "${hash2}" ]]; then
      echo "WARN: two NLB requests returned different bodies (may hit different replicas with different certs — investigate)" >&2
  else
      echo "OK: NLB attestation responses identical (consistent with shared cert material)"
  fi
fi

echo ""
echo "Manual per-instance attestation compare (private subnets):"
echo "  1. List instances: aws autoscaling describe-auto-scaling-groups --auto-scaling-group-names ${ASG_NAME}"
echo "  2. For each instance id, use SSM/Instance Connect to curl https://127.0.0.1:443${att_path:-/.well-known/enclave/attestation} -k"
echo "  3. Compare sha256 of responses — must match across replicas."
echo ""
echo "Leader lock: CloudWatch → Nitrum/${PROJECT_NAME} → LeaderLockHeld → only one InstanceId at 1 per LockKey."
echo "=== multi_replica_verify.sh passed infrastructure checks ==="
