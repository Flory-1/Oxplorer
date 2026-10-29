//! TLS for FTPS: rustls with the ring provider; certificates are checked
//! like Windows does (its certificate store, also company CAs).

use std::sync::{Arc, OnceLock};

use rustls::ClientConfig;
use rustls_platform_verifier::BuilderVerifierExt;

/// One config for all connections, so TLS sessions can be resumed (FTPS
/// servers often require the data connection to resume the control
/// connection's session).
pub(super) fn client_config() -> Result<Arc<ClientConfig>, String> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .map_err(|e| e.to_string())?
                .with_platform_verifier()
                .map_err(|e| e.to_string())?
                .with_no_client_auth();
            Ok(Arc::new(config))
        })
        .clone()
}
