const CONFIG_PATH: &str = "/app/nitrum.toml";

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
pub struct Config {
    pub tls_termination: TlsTermination,
}

pub fn load() -> Config {
    let path = std::env::var("CONFIG_PATH").unwrap_or_else(|_| CONFIG_PATH.to_string());
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read config file {path}: {e}"));
    toml::from_str(&contents)
        .unwrap_or_else(|e| panic!("failed to parse config file {path}: {e}"))
}
