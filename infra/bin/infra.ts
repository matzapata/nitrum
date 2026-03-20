#!/usr/bin/env node
import * as cdk from 'aws-cdk-lib';
import { NitrumStack } from '../lib/nitrum-stack';

const app = new cdk.App();

const required = (name: string, value: string | undefined): string => {
  if (value === undefined || value === '') {
    console.error(`Missing required env: ${name}`);
    process.exit(1);
  }
  return value;
};

const appEnv = required('DEPLOYMENT', process.env.DEPLOYMENT);
if (appEnv !== 'dev' && appEnv !== 'prod') {
  console.error('DEPLOYMENT must be either "dev" or "prod"');
  process.exit(1);
}
const account = required('CDK_DEPLOY_ACCOUNT', process.env.CDK_DEPLOY_ACCOUNT);
const region = required('CDK_DEPLOY_REGION', process.env.CDK_DEPLOY_REGION);
const eifPath = required('EIF_PATH', process.env.EIF_PATH);

new NitrumStack(app, 'NitrumStack', {
  appEnv,
  region,
  eifPath,
  cdkEnv: { account, region },
});
