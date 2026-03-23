//! Random bytes and DEK generation. Uses the rand crate (via crate::rand_crate to avoid name shadow).

use anyhow::Result;

#[cfg(feature = "enclave")]
pub fn rand_bytes(size: usize) -> Result<Vec<u8>> {
    use crate::utils::nsm::NsmConnection;
    use aws_nitro_enclaves_nsm_api as nitro;

    let nsm_conn = NsmConnection::try_new().map_err(|e| anyhow::anyhow!("NsmConnection: {e:?}"))?;
    let mut out = Vec::with_capacity(size);

    while out.len() < size {
        match nitro::driver::nsm_process_request(nsm_conn.fd(), nitro::api::Request::GetRandom) {
            nitro::api::Response::GetRandom { random } => {
                anyhow::ensure!(!random.is_empty(), "NSM GetRandom returned an empty buffer");
                let need = size - out.len();
                out.extend_from_slice(&random[..random.len().min(need)]);
            }
            nitro::api::Response::Error(e) => {
                return Err(anyhow::anyhow!(
                    "Could not get entropy from the Nitro Secure Module! {e:?}"
                ));
            }
            _ => {
                return Err(anyhow::anyhow!(
                    "Received unknown response from Nitro Secure Module"
                ));
            }
        }
    }

    Ok(out)
}

#[cfg(not(feature = "enclave"))]
pub fn rand_bytes(size: usize) -> Result<Vec<u8>> {
    use rand::RngCore;
    let mut buf = vec![0u8; size];
    rand::thread_rng().fill_bytes(&mut buf);
    Ok(buf)
}
