//! Импорт клиентов (ТЗ §4.1.1): сопоставление колонок и поиск дублей. Без ввода-вывода.

use serde::{Deserialize, Serialize};

/// Поле карточки клиента, в которое можно загрузить колонку файла.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Name,
    Kind,
    Phone,
    Email,
    Description,
    Tags,
    Manager,
}

impl Field {
    pub const ALL: [Field; 7] = [
        Field::Name,
        Field::Kind,
        Field::Phone,
        Field::Email,
        Field::Description,
        Field::Tags,
        Field::Manager,
    ];

    /// Заголовок в шаблоне.
    pub fn title(self) -> &'static str {
        match self {
            Self::Name => "Название",
            Self::Kind => "Тип",
            Self::Phone => "Телефон",
            Self::Email => "Email",
            Self::Description => "Описание",
            Self::Tags => "Теги",
            Self::Manager => "Менеджер",
        }
    }

    fn synonyms(self) -> &'static [&'static str] {
        match self {
            Self::Name => &[
                "название",
                "фио",
                "имя",
                "клиент",
                "организация",
                "компания",
                "name",
            ],
            Self::Kind => &["тип", "вид", "kind", "type"],
            Self::Phone => &["телефон", "тел", "phone", "мобильный"],
            Self::Email => &["email", "e-mail", "почта", "эл. почта", "электронная почта"],
            Self::Description => &[
                "описание",
                "комментарий",
                "примечание",
                "заметки",
                "description",
            ],
            Self::Tags => &["теги", "метки", "tags"],
            Self::Manager => &["менеджер", "ответственный", "manager"],
        }
    }
}

fn clean_header(h: &str) -> String {
    h.trim()
        .trim_start_matches('\u{feff}')
        .trim_end_matches([':', '*'])
        .trim()
        .to_lowercase()
}

/// Сопоставление по заголовкам: поле → номер колонки. Каждая колонка — не больше одному полю.
pub fn guess_mapping(headers: &[String]) -> Vec<(Field, usize)> {
    let cleaned: Vec<String> = headers.iter().map(|h| clean_header(h)).collect();
    let mut used = vec![false; headers.len()];
    let mut out = Vec::new();
    for f in Field::ALL {
        let hit = cleaned.iter().enumerate().find(|(i, h)| {
            !used[*i]
                && f.synonyms()
                    .iter()
                    .any(|s| *h == *s || h.starts_with(&format!("{s} ")))
        });
        if let Some((i, _)) = hit {
            used[i] = true;
            out.push((f, i));
        }
    }
    out
}

/// Тип клиента из текста ячейки: «ООО …», «компания», «юр» → компания; иначе физлицо.
pub fn guess_company(kind_cell: Option<&str>, name: &str) -> bool {
    if let Some(k) = kind_cell.map(str::to_lowercase) {
        let k = k.trim();
        if k.starts_with("юр") || k.starts_with("комп") || k.starts_with("орг") || k == "company"
        {
            return true;
        }
        if k.starts_with("физ") || k.starts_with("person") || k.starts_with("ип") {
            return false;
        }
    }
    let n = name.trim_start().to_lowercase();
    LEGAL_FORMS
        .iter()
        .filter(|f| **f != "ип")
        .any(|f| n.starts_with(&format!("{f} ")))
}

const LEGAL_FORMS: [&str; 7] = ["ооо", "оао", "зао", "пао", "ао", "ип", "нко"];

/// Название для сравнения дублей: без регистра, кавычек, правовой формы и лишних пробелов.
pub fn norm_name(s: &str) -> String {
    let lowered = s.to_lowercase().replace('ё', "е");
    let no_quotes: String = lowered
        .chars()
        .map(|c| {
            if "\"'«»“”„".contains(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let words: Vec<&str> = no_quotes
        .split_whitespace()
        .filter(|w| !LEGAL_FORMS.contains(w))
        .collect();
    words.join(" ")
}

/// Телефон для сравнения: только цифры, «8» в начале российского номера → «7».
pub fn norm_phone(s: &str) -> Option<String> {
    let mut d: String = s.chars().filter(char::is_ascii_digit).collect();
    if d.len() == 11 && d.starts_with('8') {
        d.replace_range(0..1, "7");
    }
    if d.len() == 10 {
        d.insert(0, '7');
    }
    (d.len() >= 7).then_some(d)
}

pub fn norm_email(s: &str) -> Option<String> {
    let e = s.trim().to_lowercase();
    e.contains('@').then_some(e)
}

/// Разделитель CSV по первой строке: `;` (Excel в России) или `,`.
pub fn delimiter(first_line: &str) -> u8 {
    let semi = first_line.matches(';').count();
    let comma = first_line.matches(',').count();
    let tab = first_line.matches('\t').count();
    if tab > semi && tab > comma {
        b'\t'
    } else if comma > semi {
        b','
    } else {
        b';'
    }
}

/// Теги из ячейки: «VIP, опт; Москва» → три тега.
pub fn split_tags(s: &str) -> Vec<String> {
    s.split([',', ';'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn mapping_by_russian_headers() {
        let m = guess_mapping(&h(&[
            "\u{feff}ФИО",
            "Тел.",
            "E-mail",
            "Комментарий",
            "Ответственный",
            "Прочее",
        ]));
        assert!(m.contains(&(Field::Name, 0)));
        assert!(m.contains(&(Field::Email, 2)));
        assert!(m.contains(&(Field::Description, 3)));
        assert!(m.contains(&(Field::Manager, 4)));
        assert!(!m.iter().any(|(_, i)| *i == 5));
    }

    #[test]
    fn duplicates_match_despite_formatting() {
        assert_eq!(norm_name("ООО «Ромашка»"), norm_name("ромашка"));
        assert_eq!(norm_name("  Пётр   Иванов "), "петр иванов");
        assert_eq!(
            norm_phone("8 (900) 123-45-67"),
            norm_phone("+7 900 1234567")
        );
        assert_eq!(norm_phone("900 123 45 67").as_deref(), Some("79001234567"));
        assert_eq!(norm_phone("123"), None);
        assert_eq!(norm_email(" Info@Firm.RU "), Some("info@firm.ru".into()));
        assert_eq!(norm_email("нет"), None);
    }

    #[test]
    fn company_or_person() {
        assert!(guess_company(None, "ООО Ромашка"));
        assert!(!guess_company(None, "ИП Петров"));
        assert!(!guess_company(None, "Иванов Иван"));
        assert!(guess_company(Some("Юр. лицо"), "Иванов"));
        assert!(!guess_company(Some("физлицо"), "ООО Ромашка"));
    }

    #[test]
    fn csv_delimiter_and_tags() {
        assert_eq!(delimiter("ФИО;Телефон;Email"), b';');
        assert_eq!(delimiter("name,phone"), b',');
        assert_eq!(delimiter("a\tb\tc"), b'\t');
        assert_eq!(split_tags("VIP, опт; ;Москва"), ["VIP", "опт", "Москва"]);
    }
}
