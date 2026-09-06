//! `tls_pin.rs`가 없는 빌드의 TLS 설정.

use std::sync::Arc;

use rustls::Error as TlsError;

pub fn client_config() -> Result<rustls::ClientConfig, TlsError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let platform = rustls_platform_verifier::Verifier::new(provider.clone())?;
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(platform))
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}
