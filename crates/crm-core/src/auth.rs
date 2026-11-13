//! Правила учётных записей: роли, логины, пароли (ТЗ §3.2–3.3).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Host,
    Manager,
    Employee,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Manager => "manager",
            Self::Employee => "employee",
        }
    }
}

impl std::str::FromStr for Role {
    type Err = AccountError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "host" => Ok(Self::Host),
            "manager" => Ok(Self::Manager),
            "employee" => Ok(Self::Employee),
            _ => Err(AccountError::UnknownRole),
        }
    }
}

pub const PASSWORD_MIN: usize = 10;
/// Верхняя граница — чтобы огромный «пароль» не грузил сервер хешированием.
pub const PASSWORD_MAX: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error("логин: 3–32 символа, латинские буквы, цифры, точка, дефис, подчёркивание")]
    BadUsername,
    #[error("имя не может быть пустым и длиннее 100 символов")]
    BadDisplayName,
    #[error("пароль должен быть не короче {PASSWORD_MIN} символов")]
    PasswordTooShort,
    #[error("пароль длиннее {PASSWORD_MAX} символов")]
    PasswordTooLong,
    #[error("пароль не должен содержать логин")]
    PasswordContainsUsername,
    #[error("неизвестная роль")]
    UnknownRole,
}

/// Логин хранится в нижнем регистре.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Username(String);

impl Username {
    pub fn parse(s: &str) -> Result<Self, AccountError> {
        let s = s.trim().to_ascii_lowercase();
        let ok_len = (3..=32).contains(&s.len());
        let ok_chars = s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
        if ok_len && ok_chars {
            Ok(Self(s))
        } else {
            Err(AccountError::BadUsername)
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub fn parse_display_name(s: &str) -> Result<String, AccountError> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > 100 {
        Err(AccountError::BadDisplayName)
    } else {
        Ok(s.to_owned())
    }
}

/// Длина считается в символах, а не байтах (кириллица — 2 байта на букву).
pub fn check_password(password: &str, username: &Username) -> Result<(), AccountError> {
    let len = password.chars().count();
    if len < PASSWORD_MIN {
        Err(AccountError::PasswordTooShort)
    } else if len > PASSWORD_MAX {
        Err(AccountError::PasswordTooLong)
    } else if password.to_lowercase().contains(username.as_str()) {
        Err(AccountError::PasswordContainsUsername)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_is_normalized_and_validated() {
        assert_eq!(
            Username::parse("  Ivan.Petrov ").unwrap().as_str(),
            "ivan.petrov"
        );
        assert!(Username::parse("ab").is_err());
        assert!(Username::parse(&"a".repeat(33)).is_err());
        assert!(
            Username::parse("иван").is_err(),
            "только латиница — логин вводят на любой раскладке устройства"
        );
        assert!(Username::parse("ivan petrov").is_err());
    }

    #[test]
    fn display_name_is_trimmed_and_bounded() {
        assert_eq!(
            parse_display_name("  Иван Петров "),
            Ok("Иван Петров".into())
        );
        assert_eq!(parse_display_name("   "), Err(AccountError::BadDisplayName));
        assert_eq!(
            parse_display_name(&"я".repeat(101)),
            Err(AccountError::BadDisplayName)
        );
    }

    #[test]
    fn password_rules() {
        let u = Username::parse("ivan").unwrap();
        assert_eq!(
            check_password("короткий", &u),
            Err(AccountError::PasswordTooShort)
        );
        assert_eq!(
            check_password("десятьбукв", &u),
            Ok(()),
            "10 кириллических символов — достаточно"
        );
        assert_eq!(
            check_password(&"x".repeat(257), &u),
            Err(AccountError::PasswordTooLong)
        );
        assert_eq!(
            check_password("my-IVAN-password", &u),
            Err(AccountError::PasswordContainsUsername)
        );
    }

    #[test]
    fn role_round_trips() {
        for r in [Role::Host, Role::Manager, Role::Employee] {
            assert_eq!(r.as_str().parse::<Role>(), Ok(r));
        }
        assert!("admin".parse::<Role>().is_err());
    }
}
