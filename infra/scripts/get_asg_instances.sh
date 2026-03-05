#!/usr/bin/env bash

set +x
set -e

aws autoscaling describe-auto-scaling-groups --region "${CDK_DEPLOY_REGION}" --auto-scaling-group-name "${1}" | jq -r '.AutoScalingGroups[0].Instances[] | select ( .LifecycleState | contains("InService")) | .InstanceId '