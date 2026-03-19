/// Compose file path relative to the project root (local dev stack).
pub const ENCLAVE_DEV_COMPOSE_FILE: &str = ".nitrum/enclave-dev-compose.yml";

/// Docker image for the Nitro CLI.
pub const NITRO_CLI_DOCKER_IMAGE: &str = "matzapata/nitrum-nitro-cli:latest";

/// Local tag for `docker build` before `compose up` (matches `${ENCLAVE_IMAGE}` in dev compose).
pub const ENCLAVE_DEV_LOCAL_IMAGE: &str = "nitrum/enclave-dev";

// Enclave base image
pub const ENCLAVE_DEV_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:dev";
pub const ENCLAVE_PROD_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:latest";

// Github repo constants
pub const NITRUM_GITHUB_REPO_OWNER: &str = "matzapata";
pub const NITRUM_GITHUB_REPO_NAME: &str = "nitrum";
