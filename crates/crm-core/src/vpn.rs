//! VPN WireGuard для работы из дома (ТЗ §9): адреса и тексты конфигураций.
//!
//! Через туннель ходит только CRM (AllowedIPs = адрес сервера): интернет
//! сотрудника идёт как обычно, домашняя сеть офису не видна.

use std::net::Ipv4Addr;

pub const SERVER_IP: Ipv4Addr = Ipv4Addr::new(10, 77, 0, 1);
pub const DEFAULT_PORT: u16 = 51820;
/// Имя туннеля в Windows (служба WireGuardTunnel$crm-vpn).
pub const TUNNEL: &str = "crm-vpn";

pub fn is_vpn_ip(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    (a, b, c) == (10, 77, 0)
}

/// Первый свободный адрес сотрудника: 10.77.0.2 … 10.77.0.254.
pub fn next_address(taken: &[Ipv4Addr]) -> Option<Ipv4Addr> {
    (2..=254)
        .map(|d| Ipv4Addr::new(10, 77, 0, d))
        .find(|a| !taken.contains(a))
}

/// Адрес, по которому сотрудники достучатся до офиса: белый IP или DDNS-имя.
pub fn parse_endpoint(s: &str) -> Result<String, &'static str> {
    let e = s.trim().trim_end_matches('.').to_lowercase();
    let ok = !e.is_empty()
        && e.len() <= 253
        && e.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !ok {
        return Err(
            "внешний адрес: белый IP (например 85.12.34.56) или имя DDNS (office.keenetic.pro)",
        );
    }
    if let Ok(ip) = e.parse::<Ipv4Addr>() {
        if ip.is_private() || ip.is_loopback() || ip.is_link_local() {
            return Err("это адрес внутренней сети — нужен внешний (белый) IP офиса или имя DDNS");
        }
    }
    Ok(e)
}

pub struct Peer<'a> {
    pub name: &'a str,
    pub public_key: &'a str,
    pub address: Ipv4Addr,
}

/// Конфигурация туннеля на сервере.
pub fn server_conf(private_key: &str, port: u16, peers: &[Peer<'_>]) -> String {
    let mut s = format!(
        "# CRM: создано автоматически, изменения вручную будут перезаписаны\n\
         [Interface]\nPrivateKey = {private_key}\nAddress = {SERVER_IP}/24\nListenPort = {port}\n"
    );
    for p in peers {
        // Имя — только в комментарии и без переводов строк.
        let name: String = p.name.chars().filter(|c| !c.is_control()).collect();
        s.push_str(&format!(
            "\n# {name}\n[Peer]\nPublicKey = {}\nAllowedIPs = {}/32\n",
            p.public_key, p.address
        ));
    }
    s
}

/// Конфигурация для сотрудника (файл или QR-код).
pub fn client_conf(
    private_key: &str,
    address: Ipv4Addr,
    server_public: &str,
    endpoint: &str,
    port: u16,
) -> String {
    format!(
        "[Interface]\nPrivateKey = {private_key}\nAddress = {address}/32\n\n\
         [Peer]\nPublicKey = {server_public}\nEndpoint = {endpoint}:{port}\n\
         AllowedIPs = {SERVER_IP}/32\nPersistentKeepalive = 25\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_fill_gaps_and_end() {
        assert_eq!(next_address(&[]), Some(Ipv4Addr::new(10, 77, 0, 2)));
        let taken = [Ipv4Addr::new(10, 77, 0, 2), Ipv4Addr::new(10, 77, 0, 4)];
        assert_eq!(next_address(&taken), Some(Ipv4Addr::new(10, 77, 0, 3)));
        let all: Vec<_> = (2..=254).map(|d| Ipv4Addr::new(10, 77, 0, d)).collect();
        assert_eq!(next_address(&all), None);
        assert!(is_vpn_ip(SERVER_IP));
        assert!(!is_vpn_ip(Ipv4Addr::new(192, 168, 1, 2)));
    }

    #[test]
    fn endpoint_must_be_public() {
        assert_eq!(
            parse_endpoint(" Office.Keenetic.PRO. "),
            Ok("office.keenetic.pro".into())
        );
        assert_eq!(parse_endpoint("85.12.34.56"), Ok("85.12.34.56".into()));
        assert!(parse_endpoint("192.168.1.1")
            .unwrap_err()
            .contains("внутренней"));
        assert!(parse_endpoint("evil\nAllowedIPs=0.0.0.0/0").is_err());
        assert!(parse_endpoint("").is_err());
    }

    #[test]
    fn client_routes_only_crm_through_tunnel() {
        let c = client_conf(
            "priv",
            Ipv4Addr::new(10, 77, 0, 5),
            "pub",
            "office.example",
            51820,
        );
        assert!(c.contains("AllowedIPs = 10.77.0.1/32"));
        assert!(c.contains("Endpoint = office.example:51820"));
        assert!(c.contains("Address = 10.77.0.5/32"));
    }

    #[test]
    fn server_conf_lists_peers_safely() {
        let peers = [Peer {
            name: "Ноутбук\nPrivateKey = x",
            public_key: "k1",
            address: Ipv4Addr::new(10, 77, 0, 2),
        }];
        let s = server_conf("srv", 51820, &peers);
        assert!(s.contains("ListenPort = 51820"));
        assert!(s.contains("AllowedIPs = 10.77.0.2/32"));
        assert_eq!(s.matches("PrivateKey").count(), 2, "{s}");
        assert!(s.contains("# НоутбукPrivateKey = x"), "имя не ломает файл");
    }
}
