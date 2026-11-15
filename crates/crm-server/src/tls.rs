//! Собственный центр сертификации сервера и сертификат HTTPS (ТЗ §3.1).
//!
//! CA создаётся один раз и переносится при переезде (§2.8) — сотрудникам не нужно
//! заново доверять серверу. Сертификат сервера перевыпускается автоматически, если
//! у компьютера появился новый адрес или до истечения осталось меньше 30 дней.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose, SanType,
};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};
use x509_parser::extensions::GeneralName;
use x509_parser::pem::parse_x509_pem;

const CA_VALIDITY: Duration = Duration::days(3650);
const CERT_VALIDITY: Duration = Duration::days(397);
const RENEW_BEFORE: Duration = Duration::days(30);

pub const CA_CERT: &str = "ca.crt";
const CA_KEY: &str = "ca.key";
const SERVER_CERT: &str = "server.crt";
const SERVER_KEY: &str = "server.key";

/// Имена и адреса, на которые выписывается сертификат.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sans {
    pub names: Vec<String>,
    pub ips: Vec<IpAddr>,
}

/// Готовый к запуску набор: PEM сертификата и ключа сервера, PEM и отпечаток CA.
#[derive(Debug, Clone)]
pub struct TlsMaterial {
    pub ca_pem: String,
    pub ca_fingerprint: String,
    pub cert_pem: String,
    pub key_pem: String,
}

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("ошибка файла {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("ошибка сертификата: {0}")]
    Cert(#[from] rcgen::Error),
}

/// Адреса этого компьютера для сертификата: localhost, имя ПК, имя.local,
/// петлевые и все сетевые адреса (кроме link-local IPv6).
pub fn sans_for(hostname: &str, interface_ips: &[IpAddr]) -> Sans {
    let host = hostname.trim().to_ascii_lowercase();
    let mut names = vec!["localhost".to_owned()];
    if !host.is_empty() && host != "localhost" {
        names.push(host.clone());
        names.push(format!("{host}.local"));
    }
    let mut ips: Vec<IpAddr> = Vec::new();
    let candidates = interface_ips.iter().copied().chain([
        IpAddr::from([127, 0, 0, 1]),
        IpAddr::from(std::net::Ipv6Addr::LOCALHOST),
    ]);
    for ip in candidates {
        let link_local = matches!(ip, IpAddr::V6(v6) if (v6.segments()[0] & 0xffc0) == 0xfe80);
        if !link_local && !ips.contains(&ip) {
            ips.push(ip);
        }
    }
    Sans { names, ips }
}

/// Добавить внешний адрес (доступ через интернет): IP — как IP, имя — как имя.
pub fn with_public(mut sans: Sans, public: Option<&str>) -> Sans {
    match public.map(str::parse::<IpAddr>) {
        Some(Ok(ip)) if !sans.ips.contains(&ip) => sans.ips.push(ip),
        Some(Err(_)) => {
            let name = public.unwrap_or_default().to_ascii_lowercase();
            if !sans.names.contains(&name) {
                sans.names.push(name);
            }
        }
        _ => {}
    }
    sans
}

/// Адреса текущего компьютера.
pub fn local_sans() -> Sans {
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    let ips: Vec<IpAddr> = if_addrs::get_if_addrs()
        .map(|list| list.into_iter().map(|i| i.ip()).collect())
        .unwrap_or_default();
    sans_for(&host, &ips)
}

/// Загружает или создаёт CA и сертификат сервера в папке `dir`. Идемпотентно.
pub fn ensure(dir: &Path, sans: &Sans, now: OffsetDateTime) -> Result<TlsMaterial, TlsError> {
    std::fs::create_dir_all(dir).map_err(io_err(dir))?;
    restrict(dir, 0o700)?;

    let (ca_key_pem, ca_pem, ca_is_new) = match (read(dir, CA_KEY)?, read(dir, CA_CERT)?) {
        (Some(k), Some(c)) => (k, c, false),
        _ => {
            let (k, c) = new_ca(now)?;
            write(dir, CA_KEY, &k, true)?;
            write(dir, CA_CERT, &c, false)?;
            (k, c, true)
        }
    };
    let ca_key = KeyPair::from_pem(&ca_key_pem)?;
    let issuer = Issuer::from_ca_cert_pem(&ca_pem, &ca_key)?;

    let existing = match (read(dir, SERVER_CERT)?, read(dir, SERVER_KEY)?) {
        (Some(c), Some(k)) if !ca_is_new && still_good(&c, sans, now) => Some((c, k)),
        _ => None,
    };
    let (cert_pem, key_pem) = match existing {
        Some(pair) => pair,
        None => {
            let key = KeyPair::generate()?;
            let cert = server_params(sans, now)?.signed_by(&key, &issuer)?;
            let pair = (cert.pem(), key.serialize_pem());
            write(dir, SERVER_KEY, &pair.1, true)?;
            write(dir, SERVER_CERT, &pair.0, false)?;
            pair
        }
    };

    Ok(TlsMaterial {
        ca_fingerprint: fingerprint(&pem_der(&ca_pem).unwrap_or_default()),
        ca_pem,
        cert_pem,
        key_pem,
    })
}

impl TlsMaterial {
    /// Цепочка для рукопожатия: сертификат сервера и CA. CA в цепочке позволяет
    /// программе сотрудника сверить его отпечаток из ключа подключения.
    pub fn chain_pem(&self) -> String {
        format!("{}\n{}", self.cert_pem.trim_end(), self.ca_pem)
    }
}

/// SHA-256 сертификата в виде `AB:CD:…` — его показывают сотруднику для сверки.
pub fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn new_ca(now: OffsetDateTime) -> Result<(String, String), TlsError> {
    let key = KeyPair::generate()?;
    let mut p = CertificateParams::default();
    let host = gethostname::gethostname().to_string_lossy().into_owned();
    p.distinguished_name = dn(&format!("CRM — центр сертификации ({host})"));
    p.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    p.not_before = now - Duration::days(1);
    p.not_after = now + CA_VALIDITY;
    let cert = p.self_signed(&key)?;
    Ok((key.serialize_pem(), cert.pem()))
}

fn server_params(sans: &Sans, now: OffsetDateTime) -> Result<CertificateParams, TlsError> {
    let mut p = CertificateParams::new(sans.names.clone())?;
    p.subject_alt_names
        .extend(sans.ips.iter().map(|ip| SanType::IpAddress(*ip)));
    p.distinguished_name = dn(sans.names.get(1).map_or("localhost", String::as_str));
    p.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    p.not_before = now - Duration::days(1);
    p.not_after = now + CERT_VALIDITY;
    Ok(p)
}

fn dn(common_name: &str) -> DistinguishedName {
    let mut d = DistinguishedName::new();
    d.push(DnType::CommonName, common_name);
    d.push(DnType::OrganizationName, "CRM");
    d
}

fn pem_der(pem: &str) -> Option<Vec<u8>> {
    parse_x509_pem(pem.as_bytes()).ok().map(|(_, p)| p.contents)
}

/// Сертификат покрывает все нужные адреса и не истекает в ближайшие 30 дней.
fn still_good(cert_pem: &str, sans: &Sans, now: OffsetDateTime) -> bool {
    let Some(der) = pem_der(cert_pem) else {
        return false;
    };
    let Ok((_, cert)) = x509_parser::parse_x509_certificate(&der) else {
        return false;
    };
    if cert.validity().not_after.to_datetime() < now + RENEW_BEFORE {
        return false;
    }
    let Ok(Some(alt)) = cert.subject_alternative_name() else {
        return false;
    };
    let mut names = Vec::new();
    let mut ips = Vec::new();
    for g in &alt.value.general_names {
        match g {
            GeneralName::DNSName(n) => names.push(n.to_ascii_lowercase()),
            GeneralName::IPAddress(b) => {
                if let Ok(a) = <[u8; 4]>::try_from(*b) {
                    ips.push(IpAddr::from(a));
                } else if let Ok(a) = <[u8; 16]>::try_from(*b) {
                    ips.push(IpAddr::from(a));
                }
            }
            _ => {}
        }
    }
    sans.names.iter().all(|n| names.contains(n)) && sans.ips.iter().all(|i| ips.contains(i))
}

fn io_err(path: &Path) -> impl Fn(std::io::Error) -> TlsError + '_ {
    move |source| TlsError::Io {
        path: path.to_owned(),
        source,
    }
}

fn read(dir: &Path, name: &str) -> Result<Option<String>, TlsError> {
    let path = dir.join(name);
    match std::fs::read_to_string(&path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_err(&path)(e)),
    }
}

/// Запись через временный файл и переименование — сбой посередине не оставит битый файл.
fn write(dir: &Path, name: &str, content: &str, secret: bool) -> Result<(), TlsError> {
    let path = dir.join(name);
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, content).map_err(io_err(&tmp))?;
    if secret {
        restrict(&tmp, 0o600)?;
    }
    std::fs::rename(&tmp, &path).map_err(io_err(&path))
}

fn restrict(path: &Path, mode: u32) -> Result<(), TlsError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
            .map_err(io_err(path))?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::Duration;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn office(extra: &[&str]) -> Sans {
        let mut ips = vec![ip("127.0.0.1"), ip("192.168.1.10")];
        ips.extend(extra.iter().map(|s| ip(s)));
        sans_for("Office-PC", &ips)
    }

    #[test]
    fn sans_cover_names_and_addresses_without_link_local() {
        let s = sans_for(
            "Office-PC",
            &[
                ip("127.0.0.1"),
                ip("192.168.1.10"),
                ip("fe80::1"),
                ip("192.168.1.10"),
            ],
        );
        assert_eq!(s.names, vec!["localhost", "office-pc", "office-pc.local"]);
        let named = with_public(s.clone(), Some("Office.Keenetic.pro"));
        assert_eq!(
            named.names.last().map(String::as_str),
            Some("office.keenetic.pro")
        );
        let by_ip = with_public(s.clone(), Some("178.206.250.161"));
        assert!(by_ip.ips.contains(&ip("178.206.250.161")));
        assert_eq!(with_public(s.clone(), None), s);
        assert_eq!(s.ips, vec![ip("127.0.0.1"), ip("192.168.1.10"), ip("::1")]);
    }

    #[test]
    fn creates_once_then_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let now = OffsetDateTime::now_utc();
        let a = ensure(dir.path(), &office(&[]), now).unwrap();
        let b = ensure(dir.path(), &office(&[]), now).unwrap();
        assert_eq!(a.ca_fingerprint, b.ca_fingerprint);
        assert_eq!(
            a.cert_pem, b.cert_pem,
            "ничего не поменялось — сертификат тот же"
        );
        assert_eq!(a.ca_fingerprint.len(), 32 * 3 - 1, "SHA-256 в виде AB:CD:…");
        assert!(a.ca_pem.contains("BEGIN CERTIFICATE"));
    }

    #[test]
    fn new_address_reissues_server_cert_but_keeps_ca() {
        let dir = tempfile::tempdir().unwrap();
        let now = OffsetDateTime::now_utc();
        let a = ensure(dir.path(), &office(&[]), now).unwrap();
        let b = ensure(dir.path(), &office(&["100.64.0.5"]), now).unwrap();
        assert_eq!(
            a.ca_fingerprint, b.ca_fingerprint,
            "CA не меняется — доверие сохраняется"
        );
        assert_ne!(a.cert_pem, b.cert_pem);
        let c = ensure(dir.path(), &office(&[]), now).unwrap();
        assert_eq!(
            b.cert_pem, c.cert_pem,
            "адрес пропал — старый сертификат его просто покрывает"
        );
    }

    #[test]
    fn expiring_cert_is_renewed() {
        let dir = tempfile::tempdir().unwrap();
        let now = OffsetDateTime::now_utc();
        let a = ensure(dir.path(), &office(&[]), now).unwrap();
        let b = ensure(dir.path(), &office(&[]), now + Duration::days(380)).unwrap();
        assert_eq!(a.ca_fingerprint, b.ca_fingerprint);
        assert_ne!(a.cert_pem, b.cert_pem);
    }

    #[cfg(unix)]
    #[test]
    fn private_keys_are_readable_only_by_server() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        ensure(dir.path(), &office(&[]), OffsetDateTime::now_utc()).unwrap();
        for f in [CA_KEY, SERVER_KEY] {
            let mode = std::fs::metadata(dir.path().join(f))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0, "{f}: {mode:o}");
        }
    }
}
