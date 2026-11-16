//! Доступ к серверу через интернет: внешний адрес офиса (белый IP или имя DDNS).
//!
//! Роутер перенаправляет TCP-порт сервера на компьютер с CRM. Шифрование и проверка
//! «это наш сервер» — сертификат CA из ключа подключения; вход — пароль и код из приложения.

use std::net::IpAddr;

/// Внешний адрес, как его ввёл владелец: убираем `https://`, `/` и порт сервера.
/// Пустая строка — доступ через интернет выключен (`Ok(None)`).
pub fn parse_address(input: &str, port: u16) -> Result<Option<String>, &'static str> {
    let Some(host) = clean_host(input, port)? else {
        return Ok(None);
    };
    let host = host.as_str();
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if is_public_ip(ip) {
            Ok(Some(ip.to_string()))
        } else {
            Err("это адрес внутренней сети — нужен внешний («белый») IP офиса или имя DDNS")
        };
    }
    if is_dns_name(host) {
        Ok(Some(host.to_owned()))
    } else {
        Err("адрес — это IP (85.12.34.56) или имя (office.keenetic.pro)")
    }
}

/// Убрать `https://`, `/` и порт (он обязан совпадать с портом сервера). Пусто — `None`.
fn clean_host(input: &str, port: u16) -> Result<Option<String>, &'static str> {
    let mut s = input.trim().to_ascii_lowercase();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(scheme) {
            s = rest.to_owned();
        }
    }
    let s = s.trim_end_matches('/');
    if s.is_empty() {
        return Ok(None);
    }
    Ok(Some(match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => {
            if p.parse::<u16>().ok() != Some(port) {
                return Err(
                    "порт должен совпадать с портом сервера — на роутере перенаправьте его же",
                );
            }
            h.to_owned()
        }
        _ => s.to_owned(),
    }))
}

/// Адрес сервера, который сотрудник ввёл сам: годится и офисный (имя компьютера,
/// 192.168.…), и внешний. Подлинность сервера проверяет сертификат, а не адрес.
pub fn manual_address(input: &str, port: u16) -> Result<String, &'static str> {
    const HINT: &str = "адрес — это IP (192.168.1.10), имя компьютера или имя в интернете";
    let host = clean_host(input, port)?.ok_or("введите адрес сервера")?;
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip.to_string());
    }
    let name_ok = host.len() <= 253
        && host.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if name_ok {
        Ok(host)
    } else {
        Err(HINT)
    }
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_documentation()
                // 100.64.0.0/10 — «серый» адрес провайдера (CGNAT): снаружи до него не достучаться.
                || (a == 100 && (64..128).contains(&b)))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80)
        }
    }
}

/// Имя в интернете: есть точка, части из букв, цифр и дефисов; не `.local`.
fn is_dns_name(s: &str) -> bool {
    s.len() <= 253
        && s.contains('.')
        && !s.ends_with(".local")
        && s.split('.').all(|part| {
            !part.is_empty()
                && part.len() <= 63
                && !part.starts_with('-')
                && !part.ends_with('-')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && s.rsplit('.')
            .next()
            .is_some_and(|tld| tld.chars().any(|c| c.is_ascii_alphabetic()))
}

/// Адрес из офисной сети (имя компьютера, `.local`, внутренний IP) — иначе через интернет.
pub fn is_office_addr(addr: &str) -> bool {
    match addr.parse::<IpAddr>() {
        Ok(ip) => !is_public_ip(ip),
        Err(_) => !addr.contains('.') || addr.ends_with(".local"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_address_allows_office_names_and_checks_port() {
        let p = |s| manual_address(s, 8443);
        assert_eq!(
            p(" https://Office.Example.ru:8443/ "),
            Ok("office.example.ru".into())
        );
        assert_eq!(p("192.168.1.10"), Ok("192.168.1.10".into()));
        assert_eq!(p("office-pc"), Ok("office-pc".into()));
        assert_eq!(p("85.12.34.56"), Ok("85.12.34.56".into()));
        assert!(p("").is_err());
        assert!(p("office.example.ru:9000").is_err(), "чужой порт");
        assert!(p("мой сервер").is_err());
        assert!(p("a b").is_err());
    }

    #[test]
    fn address_is_cleaned_and_checked() {
        let p = |s| parse_address(s, 8443);
        assert_eq!(p(""), Ok(None));
        assert_eq!(p("  "), Ok(None));
        assert_eq!(p("178.206.250.161"), Ok(Some("178.206.250.161".into())));
        assert_eq!(
            p("https://Office.Keenetic.PRO/"),
            Ok(Some("office.keenetic.pro".into()))
        );
        assert_eq!(p("crm.example.ru:8443"), Ok(Some("crm.example.ru".into())));
        assert!(p("crm.example.ru:443").unwrap_err().contains("порт"));
        for internal in [
            "192.168.1.10",
            "10.0.0.5",
            "127.0.0.1",
            "100.72.1.1",
            "172.16.3.3",
        ] {
            assert!(
                p(internal).unwrap_err().contains("внутренней"),
                "{internal}"
            );
        }
        for bad in [
            "office-pc",
            "office-pc.local",
            "crm example.ru",
            "-a.ru",
            "1.2.3",
        ] {
            assert!(p(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn office_or_internet() {
        assert!(is_office_addr("office-pc"));
        assert!(is_office_addr("office-pc.local"));
        assert!(is_office_addr("192.168.1.10"));
        assert!(!is_office_addr("178.206.250.161"));
        assert!(!is_office_addr("office.keenetic.pro"));
    }
}
