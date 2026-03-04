/// Default external ingress port.
///
/// Use 443 when TLS termination is enabled in the data-plane config (the default),
/// or 80 for plain HTTP. Override with the `INGRESS_PORT` environment variable.
pub const INGRESS_PORT: u16 = 443;
