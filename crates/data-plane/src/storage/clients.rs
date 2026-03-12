// TODO: REMOVE
//! Shared storage/KMS/leader clients built from AppConfig.

use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_kms::Client as AwsKmsClient;

use crate::config::AppConfig;
use crate::crypto::kms::Kms;

use super::{leader::Leader, StorageClient};



/// Shared clients for storage, KMS, and leader election. Built once at startup.
pub struct InfraClients {
    pub storage: StorageClient,
    pub kms: Kms,
    pub leader: Leader,
    pub config: AppConfig,
}

impl InfraClients {
    pub async fn from_config(config: AppConfig) -> Self {
        let aws_cfg = aws_config::from_env().load().await;

        let dynamo = match &config.dynamodb_endpoint {
            Some(url) => {
                let conf = aws_sdk_dynamodb::config::Builder::from(&aws_cfg)
                    .endpoint_url(url)
                    .build();
                DynamoClient::from_conf(conf)
            }
            None => DynamoClient::new(&aws_cfg),
        };

        let storage = StorageClient::new(dynamo.clone(), config.dynamodb_table.clone());
        let kms = Kms::new(AwsKmsClient::new(&aws_cfg), config.kms_key_id.clone());
        let leader = Leader::new(
            dynamo,
            config.dynamodb_table.clone(),
            config.instance_id.clone(),
        );

        Self {
            storage,
            kms,
            leader,
            config,
        }
    }
}
