//! Attestation document generation.

/// Calls the Nitro Security Module to produce a signed attestation document.
///
/// In enclave builds this talks to `/dev/nsm` via ioctl; in non-enclave builds
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

/// Dev / non-enclave: return a placeholder attestation document.
#[cfg(not(feature = "enclave"))]
pub fn get_attestation_doc(
    nonce: Option<Vec<u8>>,
    public_key: Option<Vec<u8>>,
    user_data: Option<Vec<u8>>,
) -> Result<Vec<u8>, String> {
    let nonce_str = nonce.as_ref().map_or_else(String::new, hex::encode);
    let pubkey_str = public_key.as_ref().map_or_else(String::new, hex::encode);
    let user_data_str = user_data.as_ref().map_or_else(String::new, hex::encode);
    let placeholder =
        format!("placeholder-attestation-document,{nonce_str},{pubkey_str},{user_data_str}");
    Ok(placeholder.into_bytes())
}
