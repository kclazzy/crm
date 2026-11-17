//! Подключение программы в офисе по короткому коду — без длинного ключа и интернета.
//!
//! Владелец получает код вида `K7M4-QXZ9`: первые 4 символа — номер кода (по нему
//! сервер находит запись), последние 4 — секрет. Секрет по сети не передаётся:
//! программа и сервер выполняют SPAKE2 (обмен ключами по паролю), привязанный к
//! отпечатку CA сервера. Сервер отвечает ключом подключения, зашифрованным общим
//! ключом обмена: расшифровать его может только тот, кто знает код, а поддельный
//! сервер (или подмена TLS посередине) получает лишь «не угадал» для одной попытки.

use std::fmt;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use spake2::{Ed25519Group, Identity, Password, Spake2};

/// Символы кода: Crockford Base32 — без I, L, O, U, которые путают с 1, 0 и V.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const HALF: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnrollError {
    #[error("код — 8 букв и цифр, например K7M4-QXZ9")]
    Format,
    #[error("код не подошёл — проверьте его или попросите у владельца новый")]
    Wrong,
}

/// Код подключения: номер и секрет.
#[derive(Clone, PartialEq, Eq)]
pub struct Code {
    id: String,
    secret: String,
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Секрет не попадает в журналы.
        write!(f, "Code({}-****)", self.id)
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.id, self.secret)
    }
}

impl Code {
    /// Новый код из 40 случайных бит.
    pub fn from_random(random: [u8; 5]) -> Self {
        let mut bits = random
            .iter()
            .fold(0u64, |acc, b| (acc << 8) | u64::from(*b));
        let mut chars = [0u8; 2 * HALF];
        for c in chars.iter_mut().rev() {
            *c = ALPHABET[(bits & 31) as usize];
            bits >>= 5;
        }
        let s = String::from_utf8_lossy(&chars).into_owned();
        Self {
            id: s[..HALF].to_owned(),
            secret: s[HALF..].to_owned(),
        }
    }

    /// Код, как его ввёл человек: регистр, пробелы и дефисы не важны;
    /// похожие буквы читаются как цифры (O → 0, I и L → 1).
    pub fn parse(input: &str) -> Result<Self, EnrollError> {
        let s: String = input
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            })
            .collect();
        let ok = s.len() == 2 * HALF && s.bytes().all(|b| ALPHABET.contains(&b));
        if !ok {
            return Err(EnrollError::Format);
        }
        Ok(Self {
            id: s[..HALF].to_owned(),
            secret: s[HALF..].to_owned(),
        })
    }

    /// Номер кода — открытая часть, по ней сервер находит запись.
    pub fn id(&self) -> &str {
        &self.id
    }
}

fn identities(code: &Code, fp: &str) -> (Identity, Identity) {
    let fp = crate::connkey::normalize_fp(fp);
    (
        Identity::new(format!("crm-enroll/program/{}/{fp}", code.id).as_bytes()),
        Identity::new(format!("crm-enroll/server/{}/{fp}", code.id).as_bytes()),
    )
}

/// Сторона программы: первое сообщение уже отправлено, ждём ответ сервера.
pub struct Program {
    state: Spake2<Ed25519Group>,
    fp: String,
}

/// Программа начинает обмен. `fp` — отпечаток CA, который сервер показал в TLS.
pub fn program_start(code: &Code, fp: &str) -> (Program, Vec<u8>) {
    let (a, b) = identities(code, fp);
    let (state, msg) = Spake2::<Ed25519Group>::start_a(&Password::new(&code.secret), &a, &b);
    (
        Program {
            state,
            fp: crate::connkey::normalize_fp(fp),
        },
        msg,
    )
}

/// Ответ сервера: его сообщение обмена и зашифрованный ключ подключения.
#[derive(Debug, Clone)]
pub struct Answer {
    pub msg: Vec<u8>,
    pub nonce: [u8; 24],
    pub sealed: Vec<u8>,
}

fn cipher(key: &[u8]) -> Result<XChaCha20Poly1305, EnrollError> {
    let key = key.get(..32).ok_or(EnrollError::Wrong)?;
    XChaCha20Poly1305::new_from_slice(key).map_err(|_| EnrollError::Wrong)
}

/// Сервер отвечает: `payload` (ключ подключения) шифруется общим ключом обмена.
/// Сервер не узнаёт, верен ли код у программы: каждая попытка считается.
pub fn server_answer(
    code: &Code,
    fp: &str,
    program_msg: &[u8],
    payload: &str,
    nonce: [u8; 24],
) -> Result<Answer, EnrollError> {
    let (a, b) = identities(code, fp);
    let (state, msg) = Spake2::<Ed25519Group>::start_b(&Password::new(&code.secret), &a, &b);
    let key = state.finish(program_msg).map_err(|_| EnrollError::Wrong)?;
    let sealed = cipher(&key)?
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: payload.as_bytes(),
                aad: crate::connkey::normalize_fp(fp).as_bytes(),
            },
        )
        .map_err(|_| EnrollError::Wrong)?;
    Ok(Answer { msg, nonce, sealed })
}

impl Program {
    /// Расшифровать ключ подключения. Не тот код или не тот сервер — `Wrong`.
    pub fn finish(self, answer: &Answer) -> Result<String, EnrollError> {
        let key = self
            .state
            .finish(&answer.msg)
            .map_err(|_| EnrollError::Wrong)?;
        let plain = cipher(&key)?
            .decrypt(
                XNonce::from_slice(&answer.nonce),
                Payload {
                    msg: &answer.sealed,
                    aad: self.fp.as_bytes(),
                },
            )
            .map_err(|_| EnrollError::Wrong)?;
        String::from_utf8(plain).map_err(|_| EnrollError::Wrong)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "AB:CD:EF:01";

    #[test]
    fn code_is_typed_loosely_and_shown_with_a_dash() {
        let c = Code::from_random([0x12, 0x34, 0x56, 0x78, 0x9a]);
        let shown = c.to_string();
        assert_eq!(shown.len(), 9);
        assert_eq!(shown.as_bytes()[4], b'-');
        assert_eq!(Code::parse(&shown).unwrap(), c);
        assert_eq!(
            Code::parse(&shown.to_lowercase().replace('-', " ")).unwrap(),
            c
        );
        // Похожие буквы — это цифры: «O» в коде нет, человек имел в виду 0.
        assert_eq!(Code::parse("k7m4-qxzo").unwrap().to_string(), "K7M4-QXZ0");
        assert_eq!(Code::parse("IL00 0000").unwrap().to_string(), "1100-0000");
        assert_eq!(Code::parse("K7M4-QXZ"), Err(EnrollError::Format));
        assert_eq!(Code::parse("K7M4-QXZU"), Err(EnrollError::Format));
        assert!(
            !format!("{c:?}").contains(&shown[5..]),
            "секрет не попадает в журналы"
        );
    }

    #[test]
    fn right_code_opens_the_key() {
        let code = Code::parse("K7M4-QXZ9").unwrap();
        let (program, msg) = program_start(&code, FP);
        let answer = server_answer(&code, FP, &msg, "CRM1-ключ", [7; 24]).unwrap();
        assert_eq!(program.finish(&answer).unwrap(), "CRM1-ключ");
    }

    #[test]
    fn wrong_code_or_substituted_server_gets_nothing() {
        let right = Code::parse("K7M4-QXZ9").unwrap();
        // Ошибся в секрете.
        let typo = Code::parse("K7M4-QXZ8").unwrap();
        let (program, msg) = program_start(&typo, FP);
        let answer = server_answer(&right, FP, &msg, "CRM1-ключ", [7; 24]).unwrap();
        assert_eq!(program.finish(&answer), Err(EnrollError::Wrong));
        // TLS подменили посередине: программа видит другой отпечаток.
        let (program, msg) = program_start(&right, "00:00:00:00");
        let answer = server_answer(&right, FP, &msg, "CRM1-ключ", [7; 24]).unwrap();
        assert_eq!(program.finish(&answer), Err(EnrollError::Wrong));
    }
}
