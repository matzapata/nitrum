//! Random bytes and DEK generation. Uses the rand crate (via crate::rand_crate to avoid name shadow).

use anyhow::Result;

#[cfg(feature = "enclave")]
pub fn rand_bytes(size: usize) -> Result<Vec<u8>> {
    use crate::utils::nsm::NsmConnection;
    use aws_nitro_enclaves_nsm_api as nitro;
    let nsm_conn = NsmConnection::try_new()
        .map_err(|e| anyhow::anyhow!("NsmConnection: {e:?}"))?;
    match nitro::driver::nsm_process_request(nsm_conn.fd(), nitro::api::Request::GetRandom) {
        nitro::api::Response::GetRandom { random } => {
            Ok(random.to_vec())
        }
        nitro::api::Response::Error(e) => Err(anyhow::anyhow!(
            "Could not get entropy from the Nitro Secure Module! {e:?}"
        )),
        _ => Err(anyhow::anyhow!(
            "Received unknown response from Nitro Secure Module"
        )),
    }
}

#[cfg(not(feature = "enclave"))]
pub fn rand_bytes(size: usize) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; size];
    crate::rand_crate::RngCore::fill_bytes(&mut crate::rand_crate::thread_rng(), &mut buf);
    Ok(buf)
}
