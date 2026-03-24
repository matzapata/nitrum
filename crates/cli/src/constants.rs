
// Nitrum runtime files for local development and for deployment
pub const ENCLAVE_DEV_COMPOSE_FILE: &str = ".nitrum/docker-compose.yml";
pub const NITRUM_CLOUDFORMATION_TEMPLATE_FILE: &str = ".nitrum/cloudformation-template.yml";

// Nitrum images
pub const ENCLAVE_DEV_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:dev";
pub const ENCLAVE_PROD_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:latest";

// Nitro CLI image
pub const NITRO_CLI_DOCKER_IMAGE: &str = "matzapata/nitrum-nitro-cli:latest";
