/// Controls exposure of `/.well-known/enclave/*` routes on the ingress (TLS) listener.
#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct WellKnown {
    /// When true, serve `GET /.well-known/enclave/status`.
    pub enclave_status: bool,
    /// When true, serve `GET /.well-known/enclave/attestation`.
    pub enclave_attestation: bool,
}

impl Default for WellKnown {
    fn default() -> Self {
        Self {
            enclave_status: true,
            enclave_attestation: true,
        }
    }
}
