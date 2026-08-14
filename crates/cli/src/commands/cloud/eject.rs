//! Write the bundled CloudFormation template into the project.

use crate::cloud::eject_cloud_template;
use crate::constants::ENCLAVE_CLOUD_STACK_TEMPLATE_FILE;
use crate::project::CliProject;
use anyhow::{Result, bail};
use clap::Args;
use std::path::{Component, Path, PathBuf};

#[derive(Args)]
pub struct EjectArgs {
    /// Use this project name instead of `project.name` in `nitrum.toml`
    #[arg(long = "as", value_name = "NAME")]
    pub as_name: Option<String>,
    /// Project directory (default: current directory)
    #[arg(short, long)]
    pub path: Option<PathBuf>,
    /// Overwrite the destination file if it already exists
    #[arg(long, action = clap::ArgAction::SetTrue)]
    pub force: bool,
    /// Project-relative output path (default: `infra/cloud-stack.yml`)
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

pub fn run(args: EjectArgs) -> Result<()> {
    let project = CliProject::load(args.path, args.as_name)?;
    let rel = args
        .output
        .unwrap_or_else(|| PathBuf::from(ENCLAVE_CLOUD_STACK_TEMPLATE_FILE));
    let (dest, display_rel) = resolve_eject_dest(&project.root, &rel)?;

    eject_cloud_template(&dest, args.force)?;

    println!("Wrote {display_rel}.");
    println!(
        "Point deploy at this file by adding to nitrum.toml:\n\n[cloud]\ntemplate = \"{display_rel}\"\n"
    );
    Ok(())
}

/// Resolves `--output` to a file path inside `project_root`.
fn resolve_eject_dest(project_root: &Path, rel: &Path) -> Result<(PathBuf, String)> {
    if rel.as_os_str().is_empty() || rel.is_absolute() {
        bail!("--output must be a project-relative path");
    }
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("--output must not contain `..` path segments");
    }
    let dest = normalize_components(&project_root.join(rel));
    let root = normalize_components(project_root);
    if !dest.starts_with(&root) || dest == root {
        bail!("--output must resolve inside the project directory");
    }
    let display_rel = dest
        .strip_prefix(&root)
        .unwrap_or(dest.as_path())
        .display()
        .to_string()
        .replace('\\', "/");
    Ok((dest, display_rel))
}

fn normalize_components(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::resolve_eject_dest;
    use std::path::Path;

    #[test]
    fn default_infra_path() {
        let root = Path::new("/tmp/nitrum-proj");
        let (dest, rel) = resolve_eject_dest(root, Path::new("infra/cloud-stack.yml")).expect("ok");
        assert_eq!(dest, root.join("infra/cloud-stack.yml"));
        assert_eq!(rel, "infra/cloud-stack.yml");
    }

    #[test]
    fn rejects_parent_dir() {
        let root = Path::new("/tmp/nitrum-proj");
        let err = resolve_eject_dest(root, Path::new("../outside.yml")).expect_err("..");
        assert!(err.to_string().contains("`..`"));
        let err = resolve_eject_dest(root, Path::new("infra/../../outside.yml")).expect_err("..");
        assert!(err.to_string().contains("`..`"));
    }

    #[test]
    fn rejects_absolute() {
        let root = Path::new("/tmp/nitrum-proj");
        let err = resolve_eject_dest(root, Path::new("/tmp/stack.yml")).expect_err("abs");
        assert!(err.to_string().contains("project-relative"));
    }

    #[test]
    fn rejects_empty_and_dot() {
        let root = Path::new("/tmp/nitrum-proj");
        assert!(resolve_eject_dest(root, Path::new("")).is_err());
        assert!(resolve_eject_dest(root, Path::new(".")).is_err());
    }
}
