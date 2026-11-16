//! Ключ подключения `CRM1-…` (ТЗ §2.3): всё, что нужно программе сотрудника,
//! чтобы найти сервер и доверять ему, — одной строкой от владельца.
//!
//! Внутри: имя сервера, адреса (офисная сеть, затем VPN), порт, отпечаток CA и,
//! если ключ для нового сотрудника, — одноразовое приглашение. С PIN ключ зашифрован
//! (Argon2id → XChaCha20-Poly1305): перехваченная строка без PIN бесполезна.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

pub const PREFIX: &str = "CRM1-";
const PLAIN: u8 = 0;
const WITH_PIN: u8 = 1;
const SALT: usize = 16;
const NONCE: usize = 24;

/// Содержимое ключа.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerKey {
    /// Имя сервера для списка «Мои серверы».
    pub name: String,
    /// Адреса в порядке попыток: имя в сети, .local, IP офиса, адрес VPN.
    pub addrs: Vec<String>,
    pub port: u16,
    /// SHA-256 сертификата CA сервера (hex, через двоеточие или без).
    pub fp: String,
    /// Одноразовое приглашение для нового сотрудника.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
    /// Логин приглашённого — для подсказки на экране.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Ключ узла сервера: по нему программа находит сервер без адреса
    /// (из дома, за любым роутером). `None` — владелец не включал.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// UDP-порт узла — для прямого соединения в сети офиса.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udp: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("это не ключ подключения CRM (должен начинаться с «CRM1-»)")]
    Format,
    #[error("ключ защищён PIN — введите PIN, который сообщил владелец")]
    NeedPin,
    #[error("неверный PIN или ключ повреждён")]
    BadPin,
    #[error("PIN: от 4 до 32 символов")]
    PinLength,
    #[error("внутренняя ошибка шифрования")]
    Crypto,
}

/// PIN → ключ шифрования. Параметры подобраны, чтобы перебор 6 цифр стоил дорого,
/// а одна проверка шла доли секунды.
fn derive(pin: &str, salt: &[u8]) -> Result<[u8; 32], KeyError> {
    let params = Params::new(64 * 1024, 3, 1, Some(32)).map_err(|_| KeyError::Crypto)?;
    let mut out = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pin.as_bytes(), salt, &mut out)
        .map_err(|_| KeyError::Crypto)?;
    Ok(out)
}

fn check_pin(pin: &str) -> Result<(), KeyError> {
    let n = pin.chars().count();
    if (4..=32).contains(&n) {
        Ok(())
    } else {
        Err(KeyError::PinLength)
    }
}

/// Собрать ключ. `random` — 40 случайных байт (соль + nonce) от вызывающего: ядро без ввода-вывода.
pub fn encode(
    key: &ServerKey,
    pin: Option<&str>,
    random: [u8; SALT + NONCE],
) -> Result<String, KeyError> {
    let json = serde_json::to_vec(key).map_err(|_| KeyError::Crypto)?;
    let mut out = Vec::new();
    match pin {
        None => {
            out.push(PLAIN);
            out.extend_from_slice(&json);
        }
        Some(pin) => {
            check_pin(pin)?;
            let (salt, nonce) = random.split_at(SALT);
            let k = derive(pin, salt)?;
            let cipher = XChaCha20Poly1305::new(&k.into());
            let sealed = cipher
                .encrypt(XNonce::from_slice(nonce), json.as_slice())
                .map_err(|_| KeyError::Crypto)?;
            out.push(WITH_PIN);
            out.extend_from_slice(salt);
            out.extend_from_slice(nonce);
            out.extend_from_slice(&sealed);
        }
    }
    Ok(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(out)))
}

/// Нужен ли PIN, чтобы открыть ключ.
pub fn needs_pin(s: &str) -> Result<bool, KeyError> {
    let raw = raw(s)?;
    match raw.first() {
        Some(&PLAIN) => Ok(false),
        Some(&WITH_PIN) => Ok(true),
        _ => Err(KeyError::Format),
    }
}

fn raw(s: &str) -> Result<Vec<u8>, KeyError> {
    // Ключ часто копируют с пробелами и переносами строк из письма.
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let body = compact.strip_prefix(PREFIX).ok_or(KeyError::Format)?;
    URL_SAFE_NO_PAD.decode(body).map_err(|_| KeyError::Format)
}

pub fn decode(s: &str, pin: Option<&str>) -> Result<ServerKey, KeyError> {
    let raw = raw(s)?;
    let (kind, rest) = raw.split_first().ok_or(KeyError::Format)?;
    let json = match *kind {
        PLAIN => rest.to_vec(),
        WITH_PIN => {
            let pin = pin.filter(|p| !p.is_empty()).ok_or(KeyError::NeedPin)?;
            if rest.len() < SALT + NONCE + 16 {
                return Err(KeyError::Format);
            }
            let (salt, rest) = rest.split_at(SALT);
            let (nonce, sealed) = rest.split_at(NONCE);
            let k = derive(pin.trim(), salt)?;
            XChaCha20Poly1305::new(&k.into())
                .decrypt(XNonce::from_slice(nonce), sealed)
                .map_err(|_| KeyError::BadPin)?
        }
        _ => return Err(KeyError::Format),
    };
    serde_json::from_slice(&json).map_err(|_| KeyError::Format)
}

/// Отпечаток в едином виде: строчные hex без разделителей.
pub fn normalize_fp(fp: &str) -> String {
    fp.chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ServerKey {
        ServerKey {
            name: "Офис".into(),
            addrs: vec![
                "office-pc".into(),
                "192.168.1.20".into(),
                "10.77.0.1".into(),
            ],
            port: 8443,
            fp: "AB:CD:EF".into(),
            invite: Some("tok".into()),
            user: Some("ivan".into()),
            node: Some("ab".repeat(32)),
            udp: Some(8443),
        }
    }

    #[test]
    fn key_from_older_server_has_no_node() {
        let json = br#"{"name":"A","addrs":["pc"],"port":8443,"fp":"AB"}"#;
        let mut raw = vec![PLAIN];
        raw.extend_from_slice(json);
        let k = decode(&format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(raw)), None).unwrap();
        assert_eq!((k.node, k.udp), (None, None));
    }

    #[test]
    fn plain_key_round_trips_and_survives_line_breaks() {
        let s = encode(&key(), None, [0; 40]).unwrap();
        assert!(s.starts_with(PREFIX));
        assert!(!needs_pin(&s).unwrap());
        let broken = format!("{}\n  {}", &s[..20], &s[20..]);
        assert_eq!(decode(&broken, None).unwrap(), key());
    }

    #[test]
    fn pin_key_needs_the_right_pin() {
        let s = encode(&key(), Some("4821"), [7; 40]).unwrap();
        assert!(needs_pin(&s).unwrap());
        assert_eq!(decode(&s, None), Err(KeyError::NeedPin));
        assert_eq!(decode(&s, Some("0000")), Err(KeyError::BadPin));
        assert_eq!(decode(&s, Some(" 4821 ")).unwrap(), key());
        // Содержимое не читается без PIN.
        assert!(!String::from_utf8_lossy(&raw(&s).unwrap()).contains("office"));
    }

    #[test]
    fn rejects_garbage_and_short_pins() {
        assert_eq!(decode("hello", None), Err(KeyError::Format));
        assert_eq!(decode("CRM1-!!!", None), Err(KeyError::Format));
        assert_eq!(
            encode(&key(), Some("12"), [0; 40]),
            Err(KeyError::PinLength)
        );
        let s = encode(&key(), Some("4821"), [7; 40]).unwrap();
        let cut = &s[..s.len() - 10];
        assert!(decode(cut, Some("4821")).is_err());
    }

    #[test]
    fn fingerprint_forms_compare_equal() {
        assert_eq!(normalize_fp("AB:cd:0F"), "abcd0f");
    }
}
