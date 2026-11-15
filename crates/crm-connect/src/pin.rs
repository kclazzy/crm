//! Доверие только «своему» CA: отпечаток из ключа подключения.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct Pinned {
    fp: String,
    provider: Arc<CryptoProvider>,
}

impl Pinned {
    pub fn new(fingerprint: &str) -> Self {
        Self {
            fp: crm_core::connkey::normalize_fp(fingerprint),
            provider: Arc::new(rustls::crypto::ring::default_provider()),
        }
    }
}

fn hex(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        // CA сервер присылает в цепочке; доверяем ему, только если отпечаток совпал.
        let ca = intermediates
            .iter()
            .find(|c| hex(c) == self.fp)
            .ok_or_else(|| {
                Error::General(
                    "сертификат сервера не совпадает с ключом подключения — возможно, это чужой сервер"
                        .into(),
                )
            })?;
        let mut roots = RootCertStore::empty();
        roots
            .add(ca.clone().into_owned())
            .map_err(|e| Error::General(format!("CA сервера: {e}")))?;
        let inner =
            WebPkiServerVerifier::builder_with_provider(Arc::new(roots), self.provider.clone())
                .build()
                .map_err(|e| Error::General(e.to_string()))?;
        match inner.verify_server_cert(end_entity, &[], server_name, ocsp, now) {
            // Имя/адрес мог смениться (новый IP, внешний адрес) — подпись нашим CA важнее.
            Err(Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(
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
        rustls::crypto::verify_tls13_signature(
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

/// Клиентская настройка TLS с закреплённым CA.
pub fn client_config(fingerprint: &str) -> Result<Arc<rustls::ClientConfig>, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned::new(fingerprint)))
        .with_no_client_auth();
    Ok(Arc::new(cfg))
}

pub fn server_name(addr: &str) -> Result<ServerName<'static>, Error> {
    ServerName::try_from(addr.to_owned()).map_err(|_| Error::General(format!("адрес «{addr}»")))
}
