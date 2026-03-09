#[derive(serde::Deserialize)]
pub struct TlsTermination {
    /// Whether to terminate TLS at the data-plane and forward plain HTTP to the app.
    pub enabled: bool,

    /// Use ACME (e.g. Let's Encrypt) for certificate issuance instead of self-signed.
    /// Reserved for future use.
    pub acme: bool,

    /// Domain name used for TLS certificate generation (self-signed or ACME).
    pub domain: String,
}

#[derive(serde::Deserialize)]
pub struct Egress {
    /// When false, all outbound traffic is allowed regardless of whitelist.
    pub enabled: bool,

    /// Regex patterns for allowed destination hostnames.
    /// Only evaluated when enabled = true. An empty list blocks all DNS-resolved traffic.
    pub whitelist: Vec<String>,
}

#[derive(serde::Deserialize)]
pub struct Config {
    pub tls_termination: TlsTermination,
    pub egress: Egress,
}

pub fn load(path: &std::path::Path) -> Config {
    let contents = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("failed to read config file {}: {e}", path.display()));
    toml::from_str(&contents)
        .unwrap_or_else(|e| panic!("failed to parse config file {}: {e}", path.display()))
}
