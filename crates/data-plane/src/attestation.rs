//! Attestation document generation.

/// Calls the Nitro Security Module to produce a signed attestation document.
///
/// In enclave builds this talks to `/dev/nsm` via ioctl; in non-enclave builds
/// (local dev / CI) it returns a static placeholder so the rest of the stack
/// can be exercised without real hardware.
#[cfg(feature = "enclave")]
pub fn get_attestation_doc(
    nonce: Option<Vec<u8>>,
    public_key: Option<Vec<u8>>,
    user_data: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    use aws_nitro_enclaves_nsm_api::api::{Request, Response};
    use aws_nitro_enclaves_nsm_api::driver::{nsm_exit, nsm_init, nsm_process_request};
    use serde_bytes::ByteBuf;

    let fd = nsm_init();
    if fd < 0 {
        return Err(format!("nsm_init returned {fd}"));
    }

    let request = Request::Attestation {
        user_data: user_data.map(ByteBuf::from),
        nonce: nonce.map(ByteBuf::from),
        public_key: public_key.map(ByteBuf::from),
    };

    let response = nsm_process_request(fd, request);
    nsm_exit(fd);

    match response {
        Response::Attestation { document } => Ok(document),
        other => Err(format!("unexpected NSM response: {other:?}")),
    }
}

#[cfg(not(feature = "enclave"))]
pub fn get_attestation_doc(
    _nonce: Option<Vec<u8>>,
    _public_key: Option<Vec<u8>>,
    _user_data: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    Ok(b"placeholder-attestation-document".to_vec())
}
