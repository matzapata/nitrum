import * as fs from 'fs';
import * as path from 'path';
import * as cdk from 'aws-cdk-lib';
import { Fn } from 'aws-cdk-lib';
import * as autoscaling from 'aws-cdk-lib/aws-autoscaling';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as elbv2 from 'aws-cdk-lib/aws-elasticloadbalancingv2';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as kms from 'aws-cdk-lib/aws-kms';
import * as logs from 'aws-cdk-lib/aws-logs';
import * as s3assets from 'aws-cdk-lib/aws-s3-assets';
import * as ssm from 'aws-cdk-lib/aws-ssm';
import { Construct } from 'constructs';

export interface NitrumStackProps {
  /** CDK environment (AWS account/region target) */
  cdkEnv: cdk.Environment;
  /** AWS region */
  region: string;
  /** Application environment */
  appEnv: 'dev' | 'prod';
  /** Absolute path to the EIF file to upload */
  eifPath: string;
}

export class NitrumStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: NitrumStackProps) {
    super(scope, id, { env: props.cdkEnv });

    const { appEnv, region, eifPath } = props;

    // ── VPC ───────────────────────────────────────────────────────────────────
    const vpc = new ec2.Vpc(this, 'VPC', {
      natGateways: 1,
      subnetConfiguration: [
        { name: 'public', subnetType: ec2.SubnetType.PUBLIC },
        { name: 'private', subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
      ],
      enableDnsSupport: true,
      enableDnsHostnames: true,
    });

    const privateSubnets: ec2.SubnetSelection = {
      subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS,
    };

    // ── VPC endpoints ─────────────────────────────────────────────────────────
    new ec2.InterfaceVpcEndpoint(this, 'KMSEndpoint', {
      vpc,
      subnets: privateSubnets,
      service: ec2.InterfaceVpcEndpointAwsService.KMS,
      privateDnsEnabled: true,
    });
    new ec2.InterfaceVpcEndpoint(this, 'SSMEndpoint', {
      vpc,
      subnets: privateSubnets,
      service: ec2.InterfaceVpcEndpointAwsService.SSM,
      privateDnsEnabled: true,
    });
    new ec2.InterfaceVpcEndpoint(this, 'CloudWatchLogsEndpoint', {
      vpc,
      subnets: privateSubnets,
      service: ec2.InterfaceVpcEndpointAwsService.CLOUDWATCH_LOGS,
      privateDnsEnabled: true,
    });
    new ec2.GatewayVpcEndpoint(this, 'S3Endpoint', {
      vpc,
      service: ec2.GatewayVpcEndpointAwsService.S3,
    });
    new ec2.GatewayVpcEndpoint(this, 'DynamoDBEndpoint', {
      vpc,
      service: ec2.GatewayVpcEndpointAwsService.DYNAMODB,
    });

    // ── NLB security group (required when CDK uses NLB SGs by default) ─────────
    // Without explicit rules, the NLB may block client ingress and/or egress to targets, so
    // health checks never succeed and the NLB DNS appears to hang.
    const nlbSg = new ec2.SecurityGroup(this, 'NitroNLBSG', {
      vpc,
      allowAllOutbound: true,
      description: 'NLB: internet clients in, forwarded traffic to instances',
    });
    nlbSg.addIngressRule(ec2.Peer.anyIpv4(), ec2.Port.tcp(443), 'HTTPS from internet (IPv4)');
    nlbSg.addIngressRule(ec2.Peer.anyIpv6(), ec2.Port.tcp(443), 'HTTPS from internet (IPv6)');

    // ── Security group ────────────────────────────────────────────────────────
    const enclaveSg = new ec2.SecurityGroup(this, 'NitroInstanceSG', {
      vpc,
      allowAllOutbound: true,
      description: 'Security group for Nitro Enclave EC2 instances',
    });
    enclaveSg.addIngressRule(
      ec2.Peer.ipv4(vpc.vpcCidrBlock),
      ec2.Port.tcp(443),
      'Allow HTTPS from within VPC',
    );
    enclaveSg.addIngressRule(nlbSg, ec2.Port.tcp(443), 'HTTPS from NLB (health checks + traffic)');
    enclaveSg.addIngressRule(enclaveSg, ec2.Port.tcp(443), 'Intra-SG HTTPS');
    enclaveSg.addIngressRule(enclaveSg, ec2.Port.icmpPing(), 'Intra-SG ping');
    enclaveSg.addIngressRule(ec2.Peer.anyIpv4(), ec2.Port.tcp(443), 'HTTPS (optional direct / debugging)');

    // ── KMS key ───────────────────────────────────────────────────────────────
    // Symmetric CMK: data-plane uses GenerateDataKeyWithoutPlaintext (AES-256) and
    // Decrypt with a Nitro attestation Recipient inside the enclave (kms:GenerateDataKey + kms:Decrypt).
    const enclaveKey = new kms.Key(this, 'EnclaveKey', {
      description: `Nitrum ${appEnv} enclave key — symmetric DEK wrapping (GenerateDataKey + attested Decrypt)`,
      enableKeyRotation: appEnv === 'prod',
      removalPolicy:
        appEnv === 'dev' ? cdk.RemovalPolicy.DESTROY : cdk.RemovalPolicy.RETAIN,
    });

    // ── DynamoDB table ────────────────────────────────────────────────────────
    // Single-table design; pk values: "dek" | "cert" | "lock".
    // TTL is enabled on the `ttl` attribute (used by the distributed lock).
    const enclaveTable = new dynamodb.Table(this, 'EnclaveTable', {
      tableName: `nitrum-${appEnv}`,
      partitionKey: { name: 'pk', type: dynamodb.AttributeType.STRING },
      billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
      timeToLiveAttribute: 'ttl',
      encryption: dynamodb.TableEncryption.AWS_MANAGED,
      removalPolicy:
        appEnv === 'dev' ? cdk.RemovalPolicy.DESTROY : cdk.RemovalPolicy.RETAIN,
    });

    // Plaintext params for the data-plane (SSM primary; IMDS tags remain as fallback).
    const kmsKeyParam = new ssm.StringParameter(this, 'NitrumKmsKeyParam', {
      parameterName: '/nitrum/kms_key_id',
      stringValue: enclaveKey.keyId,
      description: 'KMS key ID for Nitrum data-plane (enclave)',
    });
    const dynamoTableParam = new ssm.StringParameter(this, 'NitrumDynamoTableParam', {
      parameterName: '/nitrum/dynamodb_table',
      stringValue: enclaveTable.tableName,
      description: 'DynamoDB table name for Nitrum data-plane (enclave)',
    });

    // ── CloudWatch log group ──────────────────────────────────────────────────
    const logGroup = new logs.LogGroup(this, 'EnclaveLogGroup', {
      logGroupName: `/nitrum/${appEnv}/enclave`,
      retention: logs.RetentionDays.ONE_WEEK,
      removalPolicy:
        appEnv === 'dev' ? cdk.RemovalPolicy.DESTROY : cdk.RemovalPolicy.RETAIN,
    });

    // ── EIF asset ──────────────────────────────────────────────────────────────
    const eifAsset = new s3assets.Asset(this, 'EnclaveEifAsset', {
      path: eifPath,
    });

    // ── IAM instance role ─────────────────────────────────────────────────────
    const role = new iam.Role(this, 'InstanceRole', {
      assumedBy: new iam.ServicePrincipal('ec2.amazonaws.com'),
    });
    role.addManagedPolicy(
      iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSSMManagedInstanceCore'),
    );
    eifAsset.grantRead(role);
    enclaveKey.grant(
      role,
      'kms:GenerateDataKey', //TODO: also?
      'kms:GenerateDataKeyWithoutPlaintext',
      'kms:Decrypt',
    );
    enclaveTable.grantReadWriteData(role);
    logGroup.grantWrite(role);
    kmsKeyParam.grantRead(role);
    dynamoTableParam.grantRead(role);

    // ── User data ─────────────────────────────────────────────────────────────
    // Fn.sub replaces ${__VAR__} tokens before CloudFormation writes user data
    // to the instance. The bash script avoids ${VAR} syntax for runtime shell
    // variables to prevent conflicts with CloudFormation's substitution.
    const userDataResolved = Fn.sub(
      fs.readFileSync(path.join(__dirname, '../user_data.sh'), 'utf8'),
      {
        __REGION__: region,
        __EIF_S3_BUCKET__: eifAsset.s3BucketName,
        __EIF_S3_KEY__: eifAsset.s3ObjectKey,
        __EIF_ASSET_HASH__: eifAsset.assetHash,
      },
    );

    // ── Launch template ───────────────────────────────────────────────────────
    const launchTemplate = new ec2.LaunchTemplate(this, 'NitroLaunchTemplate', {
      // Tie the launch template identity to the EIF hash so EIF changes force
      // a concrete ASG config update on deploy.
      launchTemplateName: `nitrum-${appEnv}-${eifAsset.assetHash.slice(0, 12)}`,
      instanceType: new ec2.InstanceType('m6i.xlarge'),
      userData: ec2.UserData.custom(userDataResolved),
      nitroEnclaveEnabled: true,
      machineImage: ec2.MachineImage.latestAmazonLinux2023(),
      blockDevices: [
        {
          deviceName: '/dev/xvda',
          volume: ec2.BlockDeviceVolume.ebs(32, {
            volumeType: ec2.EbsDeviceVolumeType.GP3,
            encrypted: true,
            deleteOnTermination: appEnv === 'dev',
          }),
        },
      ],
      role,
      securityGroup: enclaveSg,
      requireImdsv2: true,
      instanceMetadataTags: true,
    });

    // ── Auto Scaling Group ────────────────────────────────────────────────────
    const asg = new autoscaling.AutoScalingGroup(this, 'NitroASG', {
      maxCapacity: 1,
      minCapacity: 1,
      desiredCapacity: 1,
      launchTemplate,
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PRIVATE_WITH_EGRESS },
      updatePolicy: autoscaling.UpdatePolicy.rollingUpdate(),
      healthChecks: autoscaling.HealthChecks.withAdditionalChecks({
        additionalTypes: [autoscaling.AdditionalHealthCheckType.ELB],
        gracePeriod: cdk.Duration.minutes(5),
      }),
    });

    // ── Network Load Balancer ─────────────────────────────────────────────────
    const nlb = new elbv2.NetworkLoadBalancer(this, 'NitroNLB', {
      internetFacing: true,
      vpc,
      vpcSubnets: { subnetType: ec2.SubnetType.PUBLIC },
      securityGroups: [nlbSg],
      crossZoneEnabled: true,
    });

    const targetGroup = new elbv2.NetworkTargetGroup(this, 'NitroTargetGroup', {
      targets: [asg],
      protocol: elbv2.Protocol.TCP,
      port: 443,
      vpc,
      healthCheck: {
        protocol: elbv2.Protocol.TCP,
        port: '443',
        interval: cdk.Duration.seconds(30),
        timeout: cdk.Duration.seconds(10),
        healthyThresholdCount: 2,
        unhealthyThresholdCount: 5,
      },
    });

    nlb.addListener('HTTPSListener', {
      port: 443,
      protocol: elbv2.Protocol.TCP,
      defaultTargetGroups: [targetGroup],
    });

    // ── Outputs ───────────────────────────────────────────────────────────────
    new cdk.CfnOutput(this, 'NLBDnsName', {
      value: nlb.loadBalancerDnsName,
      description: 'NLB DNS name - point your domain CNAME here',
    });
    new cdk.CfnOutput(this, 'KmsKeyId', {
      value: enclaveKey.keyId,
      description: 'KMS key ID',
    });
    new cdk.CfnOutput(this, 'DynamoTableName', {
      value: enclaveTable.tableName,
      description: 'DynamoDB table name',
    });
    new cdk.CfnOutput(this, 'EC2InstanceRoleARN', {
      value: role.roleArn,
      description: 'EC2 Instance Role ARN',
    });
    new cdk.CfnOutput(this, 'ASGGroupName', {
      value: asg.autoScalingGroupName,
      description: 'ASG Group Name',
    });
  }
}
