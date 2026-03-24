use anyhow::{Result, bail};

fn sanitize_bucket_label(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '.' || c == '-')
        .chars()
        .map(|c| match c {
            'A'..='Z' => c.to_ascii_lowercase(),
            'a'..='z' | '0'..='9' | '-' | '.' => c,
            _ => '-',
        })
        .collect()
}

/// Default EIF bucket: `nitrum-{project-name}` (S3 label rules, ≤63 chars).
pub fn derived_eif_bucket_name(project_name: &str) -> Result<String> {
    let slug = sanitize_bucket_label(project_name);
    if slug.is_empty() {
        bail!("`name` in nitrum.toml is empty after sanitization; set a valid project slug");
    }
    let s = format!("nitrum-{slug}");
    if !(3..=63).contains(&s.len()) {
        bail!(
            "derived S3 bucket name `{s}` is not 3–63 characters; shorten `name` in nitrum.toml"
        );
    }
    Ok(s)
}
