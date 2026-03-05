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

const deployment = required('DEPLOYMENT', process.env.DEPLOYMENT);
const account = required('CDK_DEPLOY_ACCOUNT', process.env.CDK_DEPLOY_ACCOUNT);
const region = required('CDK_DEPLOY_REGION', process.env.CDK_DEPLOY_REGION);
const appDirectory = required('APP_DIRECTORY', process.env.APP_DIRECTORY);

new NitrumStack(app, 'NitrumStack', {
  deployment,
  region,
  appDirectory,
  env: { account, region },
});
