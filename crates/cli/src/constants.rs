// Nitrum runtime files for local development and for deployment
pub fn local_stack_template() -> &'static str {
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/docker-compose.yml"))
}

pub fn cloud_stack_template() -> &'static str {
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/stack.yml"))
}

pub const ENCLAVE_LOCAL_STACK_TEMPLATE_FILE: &str = ".nitrum/local-stack.yml";
pub const ENCLAVE_CLOUD_STACK_TEMPLATE_FILE: &str = ".nitrum/cloud-stack.yml";

// Nitrum images
pub const ENCLAVE_DEV_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:dev";
pub const ENCLAVE_PROD_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:latest";

// Nitro CLI image
pub const NITRO_CLI_DOCKER_IMAGE: &str = "matzapata/nitrum-nitro-cli:latest";
