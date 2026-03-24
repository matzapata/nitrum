/// Staging directory under the project root (`template.yml` for CloudFormation deploy).
pub const NITRUM_STATE_DIR: &str = ".nitrum";
pub const NITRUM_TEMPLATE_FILE: &str = "template.yml";

/// Compose file path relative to the project root (local dev stack).
pub const ENCLAVE_DEV_COMPOSE_FILE: &str = ".nitrum/compose/enclave-dev.yml";

/// All CLI-driven `docker build` / nitro-cli `docker run` use this platform (Nitro EIF + dev/prod parity).
pub const DOCKER_PLATFORM: &str = "linux/amd64";

// TODO: make these tags dynamic based on project name
//
// Use a `matzapata/...` repository name (not `nitrum/...`): unqualified `nitrum/foo` becomes
// `docker.io/nitrum/foo`, so a cache miss makes Docker try to pull from Hub and fail.
/// Local tag for `docker build` before `compose up` (matches `${ENCLAVE_IMAGE}` in dev compose).
pub const ENCLAVE_DEV_LOCAL_IMAGE: &str = "nitrum-enclave:dev";
/// Local tag for production `docker build`; same string must be passed to `nitro-cli build-enclave --docker-uri`.
pub const ENCLAVE_PROD_LOCAL_IMAGE: &str = "nitrum-enclave:latest";

// Enclave base images
pub const ENCLAVE_DEV_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:dev";
pub const ENCLAVE_PROD_BASE_IMAGE: &str = "matzapata/nitrum-data-plane:latest";

// Nitro CLI image
pub const NITRO_CLI_DOCKER_IMAGE: &str = "matzapata/nitrum-nitro-cli:latest";

// Github repo constants
pub const NITRUM_GITHUB_REPO_OWNER: &str = "matzapata";
pub const NITRUM_GITHUB_REPO_NAME: &str = "nitrum";

/// Git ref (branch/tag) when downloading sample files from GitHub for `nitrum init`.
pub const NITRUM_GITHUB_REF: &str = "develop";
