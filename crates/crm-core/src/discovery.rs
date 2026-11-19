//! Поиск сервера в сети офиса: программа рассылает короткий запрос по UDP, сервер
//! отвечает именем, портом и отпечатком сертификата. Ответу не нужно доверять: адрес
//! только подставляется в поле, а подлинность сервера проверяет вход по коду
//! (обмен привязан к отпечатку сертификата) или ключ подключения.

use serde::{Deserialize, Serialize};

/// Порт поиска (UDP).
pub const PORT: u16 = 8444;
/// Запрос программы.
pub const REQUEST: &[u8] = b"CRM-DISCOVER 1";

/// Ответ сервера.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// Имя компьютера с сервером.
    pub name: String,
    /// Порт HTTPS.
    pub port: u16,
    /// Отпечаток сертификата (пусто, если HTTPS выключен).
    pub fp: String,
    pub version: String,
}

/// Байты ответа: «CRM-HERE 1» и JSON.
pub fn encode(reply: &Reply) -> Vec<u8> {
    let mut out = b"CRM-HERE 1\n".to_vec();
    out.extend(serde_json::to_vec(reply).unwrap_or_default());
    out
}

/// Разобрать ответ; чужие и испорченные пакеты — `None`.
pub fn decode(bytes: &[u8]) -> Option<Reply> {
    let body = bytes.strip_prefix(b"CRM-HERE 1\n")?;
    let r: Reply = serde_json::from_slice(body).ok()?;
    (!r.name.is_empty() && r.name.len() <= 255 && r.port != 0).then_some(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_round_trips_and_garbage_is_ignored() {
        let r = Reply {
            name: "OFFICE-PC".into(),
            port: 8443,
            fp: "AB:CD".into(),
            version: "0.6.0".into(),
        };
        assert_eq!(decode(&encode(&r)), Some(r.clone()));
        assert_eq!(decode(b"CRM-HERE 1\n{"), None);
        assert_eq!(decode(b"HELLO"), None);
        assert_eq!(
            decode(&encode(&Reply {
                port: 0,
                ..r.clone()
            })),
            None
        );
        assert_eq!(
            decode(&encode(&Reply {
                name: String::new(),
                ..r
            })),
            None
        );
    }
}
