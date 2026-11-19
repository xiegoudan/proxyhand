//! Root CA and on-the-fly leaf certificates for the HTTPS side of the proxy.

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls::ServerConfig;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

const CA_CN: &str = "proxyhand root CA";
/// Leaf lifetime. Browsers cap how long an interception cert may live, 397 days is what other
/// debugging proxies use.
const LEAF_DAYS: i64 = 397;
const CA_DAYS: i64 = 3650;

/// The `--dir` contents: `ca.crt` (public, meant to be installed) and `ca.key` (private).
pub struct Ca {
    cert_pem: String,
    key_pem: String,
    provider: Arc<CryptoProvider>,
    leaves: Mutex<HashMap<String, Arc<CertifiedKey>>>,
}

/// DER of `AlgorithmIdentifier { rsaEncryption, NULL }`, the prefix a PKCS#8 RSA key needs.
const RSA_ALGORITHM: [u8; 15] = [
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
];

impl Ca {
    /// Loads the CA from `dir`, generating it on first run (or when the files are unusable).
    pub fn open(dir: &Path, provider: Arc<CryptoProvider>) -> Result<Arc<Ca>, Box<dyn Error>> {
        fs::create_dir_all(dir)?;
        set_mode(dir, 0o700);
        let path = dir.join("ca.crt");
        let key_path = dir.join("ca.key");

        let existing = match (fs::read_to_string(&path), fs::read_to_string(&key_path)) {
            (Ok(cert), Ok(key)) => match Ca::from_pem(&cert, &key, provider.clone()) {
                Ok(ca) => Some(ca),
                Err(err) => {
                    eprintln!(
                        "proxyhand: existing CA in {} is unusable ({err}), regenerating",
                        dir.display()
                    );
                    None
                }
            },
            _ => None,
        };
        if let Some(ca) = existing {
            // Keep the files' permissions sane even if they were created by an older version.
            set_mode(&path, 0o644);
            set_mode(&key_path, 0o600);
            return Ok(ca);
        }

        let key = KeyPair::generate()?;
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, CA_CN);
        dn.push(DnType::OrganizationName, "proxyhand");
        dn.push(DnType::OrganizationalUnitName, "local interception");
        params.distinguished_name = dn;
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let now = OffsetDateTime::now_utc();
        params.not_before = now - Duration::days(1);
        params.not_after = now + Duration::days(CA_DAYS);
        let cert = params.self_signed(&key)?;
        let cert_pem = cert.pem();
        let key_pem = key.serialize_pem();

        fs::write(&path, &cert_pem)?;
        set_mode(&path, 0o644);
        fs::write(&key_path, &key_pem)?;
        set_mode(&key_path, 0o600);

        Ca::from_pem(&cert_pem, &key_pem, provider)
    }

    fn from_pem(
        cert_pem: &str,
        key_pem: &str,
        provider: Arc<CryptoProvider>,
    ) -> Result<Arc<Ca>, Box<dyn Error>> {
        // Parsing both halves here catches a half-written or foreign CA before it is used.
        let _issuer = Issuer::from_ca_cert_pem(cert_pem, parse_key(key_pem)?)?;
        Ok(Arc::new(Ca {
            cert_pem: cert_pem.to_string(),
            key_pem: key_pem.to_string(),
            provider,
            leaves: Mutex::new(HashMap::new()),
        }))
    }

    pub fn cert_pem(&self) -> &str {
        &self.cert_pem
    }

    /// Leaf for `host`, generating and caching it on first use.
    pub fn leaf_for(&self, host: &str) -> Option<Arc<CertifiedKey>> {
        let host = if host.is_empty() { "localhost" } else { host };
        if let Some(hit) = self
            .leaves
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(host)
        {
            return Some(hit.clone());
        }
        let key = self
            .make_leaf(host)
            .ok()
            .or_else(|| self.make_leaf("localhost").ok())?;
        let key = Arc::new(key);
        let mut leaves = self.leaves.lock().unwrap_or_else(|e| e.into_inner());
        leaves.insert(host.to_string(), key.clone());
        Some(key)
    }

    fn make_leaf(&self, host: &str) -> Result<CertifiedKey, Box<dyn Error + Send + Sync>> {
        let key = KeyPair::generate()?;
        let mut params = CertificateParams::new(vec![host.to_string()])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, host);
        params.distinguished_name = dn;
        params.use_authority_key_identifier_extension = true;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        let now = OffsetDateTime::now_utc();
        params.not_before = now - Duration::days(1);
        params.not_after = now + Duration::days(LEAF_DAYS);

        let signing_key = parse_key(&self.key_pem)?;
        let issuer = Issuer::from_ca_cert_pem(&self.cert_pem, signing_key)?;
        let cert = params.signed_by(&key, &issuer)?;
        let key_der: PrivateKeyDer<'static> = PrivatePkcs8KeyDer::from(key.serialize_der()).into();
        Ok(CertifiedKey::from_der(
            vec![cert.der().clone()],
            key_der,
            &self.provider,
        )?)
    }

    /// TLS server config handed to `tokio-rustls` for intercepted connections.
    pub fn server_config(self: &Arc<Self>) -> Arc<ServerConfig> {
        let mut cfg = ServerConfig::builder_with_provider(self.provider.clone())
            .with_safe_default_protocol_versions()
            .expect("ring supports the default protocol versions")
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(LeafResolver { ca: self.clone() }));
        cfg.alpn_protocols = vec![b"http/1.1".to_vec()];
        Arc::new(cfg)
    }

    /// SHA-256 of the CA certificate, the same form browsers show.
    pub fn fingerprint(&self) -> String {
        let der = pem_to_der(&self.cert_pem).unwrap_or_default();
        let digest = Sha256::digest(&der);
        digest
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(":")
    }
}

struct LeafResolver {
    ca: Arc<Ca>,
}

impl std::fmt::Debug for LeafResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LeafResolver")
    }
}

impl ResolvesServerCert for LeafResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let host = hello.server_name().unwrap_or("localhost");
        self.ca.leaf_for(host)
    }
}

pub fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let mut reader = std::io::Cursor::new(pem.as_bytes());
    let certs: Vec<CertificateDer<'static>> =
        rustls_pemfile::certs(&mut reader).flatten().collect();
    certs.first().map(|c| c.as_ref().to_vec())
}

/// Reads the CA private key, accepting both PKCS#8 (`PRIVATE KEY`) and PKCS#1 (`RSA PRIVATE KEY`).
///
/// rcgen's ring backend only parses PKCS#8, but the Node/forge implementation of proxyhand and
/// `openssl genrsa` both write PKCS#1, and a proxy whose CA changes invalidates every trust
/// installation, so the legacy format is wrapped in the PKCS#8 envelope instead of rejected.
fn parse_key(pem_str: &str) -> Result<KeyPair, String> {
    let pkcs8_err = match KeyPair::from_pem(pem_str) {
        Ok(key) => return Ok(key),
        Err(err) => err.to_string(),
    };
    let mut reader = std::io::Cursor::new(pem_str.as_bytes());
    match rustls_pemfile::private_key(&mut reader) {
        Ok(Some(PrivateKeyDer::Pkcs1(pkcs1))) => {
            let wrapped = PrivatePkcs8KeyDer::from(pkcs1_to_pkcs8(pkcs1.secret_pkcs1_der()));
            KeyPair::from_pkcs8_der_and_sign_algo(&wrapped, &rcgen::PKCS_RSA_SHA256)
                .map_err(|err| format!("PKCS#1 RSA key: {err}"))
        }
        Ok(Some(_)) => Err(format!("unsupported private key format ({pkcs8_err})")),
        Ok(None) => Err("no private key in ca.key".into()),
        Err(err) => Err(format!("cannot read ca.key: {err}")),
    }
}

/// Wraps a PKCS#1 `RSAPrivateKey` in the PKCS#8 `PrivateKeyInfo` that carries an RSA OID.
fn pkcs1_to_pkcs8(pkcs1: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(RSA_ALGORITHM.len() + 9 + pkcs1.len());
    body.extend_from_slice(&[0x02, 0x01, 0x00]); // version INTEGER 0
    body.extend_from_slice(&RSA_ALGORITHM);
    body.push(0x04); // OCTET STRING
    der_length(&mut body, pkcs1.len());
    body.extend_from_slice(pkcs1);
    let mut out = vec![0x30]; // SEQUENCE
    der_length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

fn der_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
        return;
    }
    let bytes = len.to_be_bytes();
    let first = bytes
        .iter()
        .position(|b| *b != 0)
        .unwrap_or(bytes.len() - 1);
    let significant = &bytes[first..];
    out.push(0x80 | significant.len() as u8);
    out.extend_from_slice(significant);
}

fn set_mode(path: &Path, mode: u32) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway 2048-bit RSA key in PKCS#1 (`RSA PRIVATE KEY`) form — what OpenSSL
    /// and most other tools emit by default.
    const PKCS1_KEY: &str = r"-----BEGIN RSA PRIVATE KEY-----
MIIEpAIBAAKCAQEA1bbzPonH8maHQNiSb54tVekn7+fJIariCS9GUqIk4o12uXE/
YFoFqB4LfUd+OjsCOwJ6YEhe1/4e2rpYxMn0EV845E+v3LmJ31KWpovXlhZ7mkB3
ZvBiuk/huuHphYwpMSORh5YYGBFhJPgTrwmsb447Uxi6gWyRl+dBAULwbAC9s0Of
gLv8PIvlwNp+vHqBlajNaqBpR3xdCxHkGxGZGuaynbbkgKVfnyKIhCFSYh+ybsSY
hV0YHYUoI/9aHhnAjcdwPZt9Bxcke7HTHHn6Ae4FOj0fDpPaeyGAe2CK7p+RG+Cj
mzhuF8RhPZ+P+8/iXBNxhe1d8T230XKYuxRq5QIDAQABAoIBADCmiDMZXGoDCxEo
/BTqtjlTbWMAqdm/M9uRZnnTLYBhYcCPVjUR6S2/QTBQwRxyaGyUgNVARE75K37x
YtnRe30k5yeaOFeP6SHshBz/wqEJFzQjLstBN8hf/Ov1F5ruMEZeaXerJGr+MdYf
nyN+mUF/6GNfQr3JwE8LNtIZse9zxSko8eSPWCmL0YE21nCbU5JBDNtR+P4p4zNj
luAPPjphqODwrCQNWeYFqTlxfBoZHHq/QHROGqt1H1u1vUeRUlgdVM/aApwy2KHD
a7Ut6CHW+hLwpAMlTT1xhvZ0HVVKEZuZGUKAbruT/Cq1CaW8BW8Z9+jViKFnwbFl
4/NohNUCgYEA8vix2mmQab3x3eFlNjxZzefp/wi6Jd9GG39A7RwibCUXBNodTert
y7wyAaBP9vjBdpu0+aM+RSfHGqCGQaIzuiIlPMcxDow4MTO18tsvC2Eqbf4khkKL
HsjwQ7ra2ZuLKMH0P7JU/2lvQwVvlrExgyfZ9pIedh8ow0jlYxRNjOsCgYEA4Syk
jvYZoNvYhi6gGgaBpiWtqYAr4/btmOHkHC+lx75n3EFres8A70VJhzJt/saKIm5b
h7EEmuQJm8RrXkzNkxXKgnGdO377NvJ1/aflGnUOrnPnMiu1icRMpqR/7kjhU3ld
REq8daFdaQ58meyBYwurjig0iELqOg9yvnDEs28CgYEA2W9X8mbQFuEmBvNDmSKv
F0Z4sIQF2giSe98IomiwIWmbzPrIiMc54T5c3fmyNgltUJCNh23gNxEe7CRFoYzf
j64o9tgnfaZcTyPAq0IdOqjMyLxLwV7hkz4tTd91JJI/Gc1goHQd/qhDKo49y3Rt
I7tRIoim+/xshT7CEUXFFNsCgYEAzdyOpdUBITtc/W/rF2ATb97Kokh31/aMylTa
w5ocKj09nvSAWsqdxCLIUj4cqezBpTDXpXqFB0asMiilMJVu7KXjetZ8NUSmT1nj
3X99yW5zebKlEA097zH0EfYar7MZrcMrLaBWJATe7Yk9CUxcDj1BHrpLLLhgg6xT
7yAR9SkCgYAYnm1n8fxAHA2QUIHW2ouU3mOfHVyoFj7wbmK3eY+w3LSn3DG7jgVC
n4MTuZREJSDAdtsB+nlHZ8R6uJ5+mZDbLhW6ZOfaJW8QfwXenjcIgmri6PIyJggh
Wh33+kd2b6ND03d1LcoF8ZstlEP8YJvPdD4yRGq42VNRSLrEqFeEIA==
-----END RSA PRIVATE KEY-----";

    fn provider() -> Arc<CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    #[test]
    fn loads_pkcs1_rsa_keys() {
        let key = parse_key(PKCS1_KEY).expect("a PKCS#1 RSA key should load");
        assert_eq!(key.algorithm(), &rcgen::PKCS_RSA_SHA256);
    }

    #[test]
    fn reuses_an_existing_pkcs1_ca() {
        let dir = tempfile::tempdir().unwrap();
        let key = parse_key(PKCS1_KEY).unwrap();
        let mut params = CertificateParams::default();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, CA_CN);
        params.distinguished_name = dn;
        let cert = params.self_signed(&key).unwrap();
        fs::write(dir.path().join("ca.crt"), cert.pem()).unwrap();
        fs::write(dir.path().join("ca.key"), PKCS1_KEY).unwrap();

        let ca = Ca::open(dir.path(), provider()).unwrap();

        // The CA must be adopted, not replaced: a rotated root breaks every existing trust install.
        assert_eq!(
            fs::read_to_string(dir.path().join("ca.crt")).unwrap(),
            cert.pem()
        );
        assert!(ca.leaf_for("example.com").is_some());
    }
}
