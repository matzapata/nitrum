# Hello sample

Nitrum hello enclave sample. Infra can be deployed with **CDK** (`samples/hello/.nitrum/infra`) or with the **CloudFormation template** at **`infra/cloudformation/template.yml`** (bundled into **`nitrum deploy`** from the monorepo). The template uses a slug **`EnvironmentName`** (e.g. `nitrum-staging`, `myproject-staging`), optional **`Retain`** for deletion behavior, **Metadata** (MIT-0), and **no fixed IAM physical names**. Networking and NLB behavior align with `infra/lib/nitrum-stack.ts`. **SSM parameters** for the data-plane are **`/nitrum/kms_key_id`** and **`/nitrum/dynamodb_table`** in both CDK and this template (only one stack per account/region should own those names). See the comparison table below.

## Prerequisites

- AWS CLI configured (`aws sts get-caller-identity`).
- Choose an **`EnvironmentName`** slug: **lowercase**, starts with a letter, then letters, digits, or hyphens (pattern `^[a-z][a-z0-9-]{2,127}$`). Examples: `nitrum-dev`, `nitrum-staging`, `myproject-staging`. It becomes the **DynamoDB table name**, the **KMS alias** (`alias/<EnvironmentName>-enclave`), **CloudWatch log group** path segment, and **`LaunchTemplateName`** prefix. SSM keys **`/nitrum/kms_key_id`** and **`/nitrum/dynamodb_table`** are fixed (not namespaced by `EnvironmentName`).
- **Build the hello Docker image** as usual; **`nitrum.toml`** is in the image and the data-plane reads the default SSM paths above (`crates/data-plane/src/utils/ssm.rs`). Override with **`NITRUM_SSM_PARAMETER_NAMES`** or per-parameter env vars only if you use non-default paths.
- An S3 bucket in the same account/region you deploy to (create one in **Build and upload the EIF**).
- A built **EIF** (the template does not upload it).

## Build and upload the EIF

The data-plane loads **`/nitrum/kms_key_id`** and **`/nitrum/dynamodb_table`** by default (`crates/data-plane/src/utils/ssm.rs`), matching this template.

Example build for `EnvironmentName=nitrum-staging` (image tag only; no SSM build args required):

```bash
export ENV_SLUG=nitrum-staging

docker build \
  --platform linux/amd64 \
  -f samples/hello/Dockerfile \
  -t "matzapata/nitrum-hello:${ENV_SLUG}" \
  samples/hello

# Then run your usual nitro-cli / enclave build to produce enclave.eif
```

Upload the EIF:

**Using the Nitrum CLI:** `nitrum deploy` uses **`nitrum-{name}`** as the EIF bucket (from top-level **`name`** in `nitrum.toml`), then runs **`aws s3 mb`** before **`aws s3 cp`** when the bucket is missing—the same pattern as the shell snippet below.

Create a bucket manually in the **same region** you will deploy to if you are not using the CLI for that step. Names must be **globally unique** in S3; pick a unique **`name`** in `nitrum.toml` if `mb` fails with a name collision.

```bash
export AWS_REGION="${AWS_REGION:-us-east-1}"
# Same as top-level `name` in nitrum.toml (CloudFormation `EnvironmentName`; SSM paths are global /nitrum/kms_key_id).
export PROJECT_NAME="nitrum-hello"
export EIF_BUCKET="nitrum-${PROJECT_NAME}"

aws s3 mb "s3://${EIF_BUCKET}" --region "${AWS_REGION}"
```

If `mb` fails because the name exists, change **`name`** in `nitrum.toml` (or use a different bucket name only when not using `nitrum deploy`). New buckets use Block Public Access by default; no extra step is required for this flow.

Then upload the EIF:

```bash
export EIF_HASH="$(sha256sum enclave.eif | cut -c1-12)"

# Object key matches `nitrum deploy` (first 12 hex chars of the EIF SHA-256).
aws s3 cp enclave.eif "s3://${EIF_BUCKET}/${EIF_HASH}" --region "${AWS_REGION}"
```

Pass the same **`EIF_HASH`** as **`EifS3Key`** and **`EifVersionLabel`** on every deploy that should roll instances to a **new** binary.

## Deploy the stack (CloudFormation)

```bash
export AWS_REGION="${AWS_REGION:-us-east-1}"
export ENV_SLUG=nitrum-hello
export STACK_NAME="${ENV_SLUG}"
export EIF_BUCKET="nitrum-${ENV_SLUG}"
export EIF_HASH="$(sha256sum enclave.eif | cut -c1-12)"

aws cloudformation deploy \
  --region "$AWS_REGION" \
  --stack-name "$STACK_NAME" \
  --template-file infra/cloudformation/template.yml \
  --capabilities CAPABILITY_IAM \
  --parameter-overrides \
    EnvironmentName="${ENV_SLUG}" \
    Retain=false \
    EifS3Bucket="$EIF_BUCKET" \
    EifS3Key="$EIF_HASH" \
    EifVersionLabel="$EIF_HASH" \
    AsgMinSize=1 \
    AsgMaxSize=1 \
    AsgDesiredCapacity=1
```

- **`Retain=false`** (default): on stack **delete**, KMS, DynamoDB, log group, and SSM parameters are **removed** (typical dev).
- **`Retain=true`**: those resources use **DeletionPolicy: Retain** (typical production); you must delete them manually if you no longer need them. **Key rotation** and **non-deleting root EBS** follow the same flag.

With **`nitrum deploy`**, pass **`--retain`** to set **`Retain=true`** (omit for **`false`**).

Read outputs (including **`NitrumSsmPrefix`** for future EIF builds):

```bash
aws cloudformation describe-stacks \
  --region "$AWS_REGION" \
  --stack-name "$STACK_NAME" \
  --query "Stacks[0].Outputs" \
  --output table
```

HTTPS via NLB:

```bash
NLB="$(aws cloudformation describe-stacks --region "$AWS_REGION" --stack-name "$STACK_NAME" \
  --query "Stacks[0].Outputs[?OutputKey=='NLBDnsName'].OutputValue" --output text)"
curl -sk "https://${NLB}/health"
```

### CDK vs this template

| Topic | CDK (`infra/`) | `infra/cloudformation/template.yml` |
|--------|----------------|----------------|
| Env dimension | **`AppEnv`** `dev` / `prod` | **`EnvironmentName`** slug (`nitrum-staging`, …) |
| Deletion / retain | `RemovalPolicy` from `appEnv` | **`Retain`** `true` / `false` (default **false**) |
| DynamoDB table | `nitrum-${appEnv}` | **`EnvironmentName`** (table name = slug) |
| SSM paths | `/nitrum/kms_key_id`, `/nitrum/dynamodb_table` | Same (global names; one stack per account/region) |
| KMS alias | (none in stack) | **`alias/${EnvironmentName}-enclave`** |
| EIF in S3 | `s3assets.Asset` on deploy | You **`aws s3 cp`**, then deploy |
| VPC AZs | Often **3** subnets | **2** AZs (2-AZ–safe regions) |

**User data** in the template mirrors `infra/user_data.sh`. If you change that script for CDK, update the embedded block in **`infra/cloudformation/template.yml`** (`Content-Type: multipart/mixed`).

## Scale up

Raise **`AsgMaxSize`** / **`AsgDesiredCapacity`** (and pass the same **`EnvironmentName`** / **`Retain`** / EIF params as on create):

```bash
aws cloudformation deploy \
  --region "$AWS_REGION" \
  --stack-name "$STACK_NAME" \
  --template-file infra/cloudformation/template.yml \
  --capabilities CAPABILITY_IAM \
  --parameter-overrides \
    EnvironmentName="${ENV_SLUG}" \
    Retain=false \
    EifS3Bucket="$EIF_BUCKET" \
    EifS3Key="$EIF_HASH" \
    EifVersionLabel="$EIF_HASH" \
    AsgMinSize=1 \
    AsgMaxSize=3 \
    AsgDesiredCapacity=2
```

Or use **`aws autoscaling set-desired-capacity`** if **`MaxSize`** already allows it (see earlier README flow).

## Scale down

Same as before: lower capacities via **`deploy`** or **`set-desired-capacity`**; set **`AsgMinSize=0`** if you want **zero** instances.

## Update to a new EIF

1. Recompute **`EIF_HASH`** from the new **`enclave.eif`** and `aws s3 cp` to **`s3://${EIF_BUCKET}/${EIF_HASH}`** (same key for **`EifS3Key`** and **`EifVersionLabel`** so **`LaunchTemplateName`** changes).
2. **`deploy`** again with the same **`EnvironmentName`** (and EIF params) so instances pick up the new object.

## Destroy

```bash
aws cloudformation delete-stack --region "$AWS_REGION" --stack-name "$STACK_NAME"
aws cloudformation wait stack-delete-complete --region "$AWS_REGION" --stack-name "$STACK_NAME"
```

If **`Retain=true`**, KMS, DynamoDB, the log group, and SSM parameters may **remain**; delete them in the console or CLI when appropriate.

## Operational tips

- SSM Session Manager: use output **`ASGGroupName`** to find instance IDs.
- **`docker logs control-plane`** on the host for gvproxy / control-plane.
- NLB is **TCP 443 passthrough**; **`curl -k`** if the cert does not match the NLB DNS name.
