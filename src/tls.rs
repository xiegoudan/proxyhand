//! Upstream TLS: the certificates proxyhand trusts when talking *to* a server.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};

/// Both configurations the proxy may dial with.
pub struct UpstreamTls {
    pub verifying: Arc<ClientConfig>,
    pub insecure: Arc<ClientConfig>,
    /// Trust anchors in the verifying store (bundled roots + OS store + --upstream-ca).
    pub root_count: usize,
}

impl UpstreamTls {
    pub fn build(
        provider: Arc<CryptoProvider>,
        system_ca: bool,
        extra_cas: &[String],
    ) -> Result<UpstreamTls, String> {
        let mut roots = RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        if system_ca {
            // Browsers use the OS store; rustls does not unless we ask.
            let native = rustls_native_certs::load_native_certs();
            for cert in native.certs {
                let _ = roots.add(cert);
            }
        }

        for (index, pem) in extra_cas.iter().enumerate() {
            let mut reader = std::io::Cursor::new(pem.as_bytes());
            let certs: Vec<CertificateDer<'static>> =
                rustls_pemfile::certs(&mut reader).flatten().collect();
            if certs.is_empty() {
                return Err(format!(
                    "--upstream-ca #{} contains no certificate",
                    index + 1
                ));
            }
            for cert in certs {
                roots
                    .add(cert)
                    .map_err(|e| format!("--upstream-ca #{}: {e}", index + 1))?;
            }
        }

        let root_count = roots.len();
        let verifying = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();

        let insecure = ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoVerify { provider }))
            .with_no_client_auth();

        Ok(UpstreamTls {
            verifying: Arc::new(with_alpn(verifying)),
            insecure: Arc::new(with_alpn(insecure)),
            root_count,
        })
    }
}

fn with_alpn(mut cfg: ClientConfig) -> ClientConfig {
    cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
    cfg
}

/// Accepts any certificate: used for the single retry when verification fails and the user did
/// not ask for strict verification.
#[derive(Debug)]
struct NoVerify {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
