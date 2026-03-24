use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// Bundled Nitrum sample CloudFormation template (same source as `samples/hello/template.yml`).
pub fn bundled_template_yaml() -> &'static str {
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../samples/hello/template.yml"
    ))
}

/// Writes the bundled template next to the project for inspection; returns the path.
pub fn write_bundled_template(project_root: &Path) -> Result<PathBuf> {
    let dir = project_root.join(crate::constants::NITRUM_STATE_DIR);
    std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = dir.join(crate::constants::NITRUM_TEMPLATE_FILE);
    std::fs::write(&path, bundled_template_yaml())
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// First 12 lowercase hex chars of the file SHA-256 (for `EifVersionLabel`).
pub fn eif_version_label(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut buf)
            .with_context(|| format!("read {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let hex = format!("{:x}", digest);
    Ok(hex[..12].to_string())
}
