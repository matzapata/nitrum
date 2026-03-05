import * as fs from 'fs';
import * as path from 'path';
import * as cdk from 'aws-cdk-lib';
import { Fn } from 'aws-cdk-lib';
import * as autoscaling from 'aws-cdk-lib/aws-autoscaling';
import * as ec2 from 'aws-cdk-lib/aws-ec2';
import * as ecrAssets from 'aws-cdk-lib/aws-ecr-assets';
import * as elbv2 from 'aws-cdk-lib/aws-elasticloadbalancingv2';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as kms from 'aws-cdk-lib/aws-kms';
import * as logs from 'aws-cdk-lib/aws-logs';
import { Construct } from 'constructs';

export interface NitrumStackProps extends cdk.StackProps {
  deployment: string;
  region: string;
  /** Absolute path to the directory containing the enclave Dockerfile */
  appDirectory: string;
}

export class NitrumStack extends cdk.Stack {
  constructor(scope: Construct, id: string, props: NitrumStackProps) {
    super(scope, id, props);

    const { deployment, region, appDirectory } = props;

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
    new ec2.InterfaceVpcEndpoint(this, 'ECREndpoint', {
      vpc,
      subnets: privateSubnets,
      service: ec2.InterfaceVpcEndpointAwsService.ECR,
      privateDnsEnabled: true,
    });
    new ec2.InterfaceVpcEndpoint(this, 'ECRDockerEndpoint', {
      vpc,
      subnets: privateSubnets,
      service: ec2.InterfaceVpcEndpointAwsService.ECR_DOCKER,
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

    // ── Security group ────────────────────────────────────────────────────────
    const enclaveSg = new ec2.SecurityGroup(this, 'NitroInstanceSG', {
      vpc,
      allowAllOutbound: true,
      description: 'Security group for Nitro Enclave EC2 instances',
    });
    enclaveSg.addIngressRule(
      ec2.Peer.ipv4(vpc.vpcCidrBlock),
      ec2.Port.tcp(443),
      'Allow HTTPS from within VPC (NLB health check)',
    );
    enclaveSg.addIngressRule(enclaveSg, ec2.Port.tcp(443), 'Intra-SG HTTPS');
    enclaveSg.addIngressRule(enclaveSg, ec2.Port.icmpPing(), 'Intra-SG ping');
    enclaveSg.addIngressRule(
      ec2.Peer.anyIpv4(),
      ec2.Port.tcp(443),
      'Allow HTTPS inbound from NLB',
    );

    // ── KMS key ───────────────────────────────────────────────────────────────
    // RSA-2048 for attestation-based decrypt: KMS releases plaintext only when
    // the enclave's PCR measurements match the key policy.
    const kmsKey = new kms.Key(this, 'EnclaveKey', {
      keySpec: kms.KeySpec.RSA_2048,
      keyUsage: kms.KeyUsage.ENCRYPT_DECRYPT,
      description: 'Nitrum enclave key - attestation-based decrypt',
      removalPolicy:
        deployment === 'dev' ? cdk.RemovalPolicy.DESTROY : cdk.RemovalPolicy.RETAIN,
    });

    // ── CloudWatch log group ──────────────────────────────────────────────────
    const logGroup = new logs.LogGroup(this, 'EnclaveLogGroup', {
      logGroupName: `/nitrum/${deployment}/enclave`,
      retention: logs.RetentionDays.ONE_WEEK,
      removalPolicy:
        deployment === 'dev' ? cdk.RemovalPolicy.DESTROY : cdk.RemovalPolicy.RETAIN,
    });

    // ── Docker image assets ───────────────────────────────────────────────────
    // Enclave image: customer app + nitrum data-plane.
    const enclaveImage = new ecrAssets.DockerImageAsset(this, 'EnclaveImage', {
      directory: appDirectory,
      platform: ecrAssets.Platform.LINUX_AMD64,
      assetName: 'nitrum-enclave',
      buildArgs: {
        DATA_PLANE_IMAGE: 'matzapata/nitrum-data-plane:latest',
      },
    });

    // ── IAM instance role ─────────────────────────────────────────────────────
    const role = new iam.Role(this, 'InstanceRole', {
      assumedBy: new iam.ServicePrincipal('ec2.amazonaws.com'),
    });
    role.addManagedPolicy(
      iam.ManagedPolicy.fromAwsManagedPolicyName('AmazonSSMManagedInstanceCore'),
    );
    kmsKey.grant(role, 'kms:Decrypt', 'kms:GetPublicKey');
    enclaveImage.repository.grantPull(role);
    logGroup.grantWrite(role);

    // ── User data ─────────────────────────────────────────────────────────────
    // Fn.sub replaces ${__VAR__} tokens before CloudFormation writes user data
    // to the instance. The bash script avoids ${VAR} syntax for runtime shell
    // variables to prevent conflicts with CloudFormation's substitution.
    const userDataResolved = Fn.sub(
      fs.readFileSync(path.join(__dirname, '../user_data.sh'), 'utf8'),
      {
        __REGION__: region,
        __ENCLAVE_IMAGE_URI__: enclaveImage.imageUri,
      },
    );

    // ── Launch template ───────────────────────────────────────────────────────
    const launchTemplate = new ec2.LaunchTemplate(this, 'NitroLaunchTemplate', {
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
            deleteOnTermination: deployment === 'dev',
          }),
        },
      ],
      role,
      securityGroup: enclaveSg,
      requireImdsv2: true,
    });

    // ── Auto Scaling Group ────────────────────────────────────────────────────
    const asg = new autoscaling.AutoScalingGroup(this, 'NitroASG', {
      maxCapacity: 2,
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
    });

    nlb.addListener('HTTPSListener', {
      port: 443,
      protocol: elbv2.Protocol.TCP,
      defaultTargetGroups: [
        new elbv2.NetworkTargetGroup(this, 'NitroTargetGroup', {
          targets: [asg],
          protocol: elbv2.Protocol.TCP,
          port: 443,
          vpc,
        }),
      ],
    });

    // ── Outputs ───────────────────────────────────────────────────────────────
    new cdk.CfnOutput(this, 'NLBDnsName', {
      value: nlb.loadBalancerDnsName,
      description: 'NLB DNS name - point your domain CNAME here',
    });
    new cdk.CfnOutput(this, 'KmsKeyId', {
      value: kmsKey.keyId,
      description: 'KMS key used for attestation-based decrypt',
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
