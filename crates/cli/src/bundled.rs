//! Files embedded at build time from the repo (no GitHub downloads at runtime).

macro_rules! include_repo {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../", $path))
    };
}

/// Nitrum CloudFormation template (`infra/cloudformation/template.yml`).
pub fn cloudformation_template_yml() -> &'static str {
    include_repo!("infra/cloudformation/template.yml")
}

/// Local dev stack (`infra/docker/compose.yml`).
pub fn docker_compose_yml() -> &'static str {
    include_repo!("infra/docker/compose.yml")
}

pub fn sample_main_js() -> &'static str {
    include_repo!("samples/hello/src/main.js")
}

pub fn sample_package_json() -> &'static str {
    include_repo!("samples/hello/package.json")
}

pub fn sample_dockerfile() -> &'static str {
    include_repo!("samples/hello/Dockerfile")
}
