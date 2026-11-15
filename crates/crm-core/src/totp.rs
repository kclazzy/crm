//! Второй фактор входа: одноразовые коды из приложения-аутентификатора (TOTP, RFC 6238)
//! и правило, когда код нужен.
//!
//! Код — 6 цифр, меняется каждые 30 секунд; принимаем соседние интервалы (часы телефона
//! могут спешить) и не принимаем один и тот же интервал дважды.

use hmac::{Hmac, Mac};
use sha1::Sha1;

pub const STEP_SECS: u64 = 30;
const DIGITS: u32 = 6;
/// Секрет: 160 бит, как рекомендует RFC 4226.
pub const SECRET_LEN: usize = 20;

/// Код для интервала `step` (время / 30 с).
pub fn code_at(secret: &[u8], step: u64) -> u32 {
    #[allow(clippy::expect_used)]
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC принимает ключ любой длины");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = usize::from(h[h.len() - 1] & 0x0f);
    let bin = u32::from_be_bytes([h[off] & 0x7f, h[off + 1], h[off + 2], h[off + 3]]);
    bin % 10u32.pow(DIGITS)
}

/// Проверить введённый код. Возвращает интервал, который совпал, — его нужно запомнить,
/// чтобы тот же код нельзя было использовать повторно.
pub fn verify(secret: &[u8], input: &str, now_secs: u64, last_step: Option<u64>) -> Option<u64> {
    let digits: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if digits.len() != DIGITS as usize || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let want: u32 = digits.parse().ok()?;
    let now = now_secs / STEP_SECS;
    [now.saturating_sub(1), now, now + 1]
        .into_iter()
        .filter(|s| last_step.is_none_or(|l| *s > l))
        .find(|s| code_at(secret, *s) == want)
}

/// Base32 без `=` — так секрет вводят в приложение вручную.
pub fn base32(bytes: &[u8]) -> String {
    const ABC: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut buf, mut bits) = (0u32, 0u32);
    for b in bytes {
        buf = (buf << 8) | u32::from(*b);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(ABC[((buf >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(ABC[((buf << (5 - bits)) & 31) as usize]));
    }
    out
}

pub fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let (mut buf, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.chars().filter(|c| *c != '=' && !c.is_whitespace()) {
        let v = match c.to_ascii_uppercase() {
            c @ 'A'..='Z' => c as u32 - 'A' as u32,
            c @ '2'..='7' => c as u32 - '2' as u32 + 26,
            _ => return None,
        };
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

/// Ссылка для QR-кода: приложение само заполнит название и логин.
pub fn otpauth_url(issuer: &str, account: &str, secret: &[u8]) -> String {
    let enc = |s: &str| -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' => {
                    char::from(b).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect()
    };
    format!(
        "otpauth://totp/{}:{}?secret={}&issuer={}&algorithm=SHA1&digits={DIGITS}&period={STEP_SECS}",
        enc(issuer),
        enc(account),
        base32(secret),
        enc(issuer)
    )
}

/// Когда спрашивать код.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Только у тех, кто сам включил код, и только при входе из интернета.
    Off,
    /// Из интернета — всегда с кодом; в офисе — без кода (по умолчанию).
    Remote,
    /// Всегда с кодом, и в офисе тоже.
    All,
}

impl Policy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Remote => "remote",
            Self::All => "all",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "off" => Some(Self::Off),
            "remote" => Some(Self::Remote),
            "all" => Some(Self::All),
            _ => None,
        }
    }
}

/// Что делать после верного пароля.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Login {
    /// Входит без кода.
    Allow,
    /// Нужен код из приложения (или резервный).
    NeedCode,
    /// Входит, но сначала настраивает код — до этого остальное недоступно.
    Setup,
    /// Из интернета с одним паролем нельзя, а кода ещё нет: иначе вор пароля
    /// привязал бы свой телефон. Настроить — в офисе.
    Refuse,
}

pub fn login_rule(policy: Policy, enrolled: bool, from_office: bool) -> Login {
    let required = match policy {
        Policy::All => true,
        Policy::Remote => !from_office,
        Policy::Off => false,
    };
    match (enrolled, required) {
        (true, _) if from_office && policy != Policy::All => Login::Allow,
        (true, _) => Login::NeedCode,
        (false, false) => Login::Allow,
        (false, true) if from_office => Login::Setup,
        (false, true) => Login::Refuse,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // RFC 6238, приложение B: секрет «12345678901234567890», SHA-1, последние 6 цифр.
    const RFC: &[u8] = b"12345678901234567890";

    #[test]
    fn rfc6238_vectors() {
        for (t, code) in [
            (59, 287_082),
            (1_111_111_109, 81_804),
            (1_111_111_111, 50_471),
            (1_234_567_890, 5_924),
            (2_000_000_000, 279_037),
        ] {
            assert_eq!(code_at(RFC, t / STEP_SECS), code, "t={t}");
        }
    }

    #[test]
    fn verify_accepts_neighbours_once() {
        let t = 1_111_111_111;
        let code = format!("{:06}", code_at(RFC, t / STEP_SECS));
        let step = verify(RFC, &code, t, None).unwrap();
        assert_eq!(
            verify(RFC, &code, t, Some(step)),
            None,
            "повтор того же кода"
        );
        assert_eq!(
            verify(RFC, &code, t + 30, None),
            Some(step),
            "часы спешат на 30 с"
        );
        assert_eq!(verify(RFC, &code, t + 90, None), None, "слишком старый");
        assert_eq!(
            verify(RFC, " 050 471 ", t, None),
            Some(step),
            "пробелы не мешают"
        );
        assert_eq!(verify(RFC, "5047", t, None), None);
        assert_eq!(verify(RFC, "abcdef", t, None), None);
    }

    #[test]
    fn base32_roundtrip_and_url() {
        assert_eq!(base32(b"foobar"), "MZXW6YTBOI");
        assert_eq!(base32_decode("MZXW6YTBOI").unwrap(), b"foobar");
        let url = otpauth_url("CRM Офис", "ivan", RFC);
        assert!(url.starts_with("otpauth://totp/CRM%20%D0%9E"), "{url}");
        assert!(
            url.contains("secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"),
            "{url}"
        );
    }

    #[test]
    fn when_code_is_needed() {
        use Login::*;
        use Policy::*;
        // (политика, код настроен, из офиса) → решение
        for (p, enrolled, office, want) in [
            (Off, false, false, Allow),
            (Off, true, true, Allow),
            (Off, true, false, NeedCode),
            (Remote, false, true, Allow),
            (Remote, false, false, Refuse),
            (Remote, true, true, Allow),
            (Remote, true, false, NeedCode),
            (All, false, true, Setup),
            (All, false, false, Refuse),
            (All, true, true, NeedCode),
            (All, true, false, NeedCode),
        ] {
            assert_eq!(
                login_rule(p, enrolled, office),
                want,
                "{p:?} {enrolled} {office}"
            );
        }
    }
}
