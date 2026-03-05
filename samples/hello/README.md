
# Hello sample

Build with: `just build`

Run with: `just up`

Run e2e tests with: `just e2e`

Deploy with cdk

```
export DEPLOYMENT=dev
export CDK_DEPLOY_REGION=sa-east-1
export CDK_DEPLOY_ACCOUNT=$(aws sts get-caller-identity | jq -r '.Account')
export APP_DIRECTORY=$(pwd)

cd ../../infra && cdk deploy NitrumStack -O out.json --require-approval never
```