//! Interface language. English is the default; Spanish is selected with `language = "es"`
//! in the config file or the `PISTASHO_LANG` environment variable (the variable wins).
//! `language = "auto"` detects it from the system locale (`LC_ALL`, `LC_MESSAGES`, `LANG`).
//!
//! Strings are written inline at the call site as `tr("English", "Español")`.

use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Es,
}

static LANG: AtomicU8 = AtomicU8::new(0);

impl Lang {
    /// `es`, `es_MX`, `ES`… → Spanish; anything else → English.
    pub fn parse(s: &str) -> Self {
        if s.trim().to_ascii_lowercase().starts_with("es") {
            Lang::Es
        } else {
            Lang::En
        }
    }
}

pub fn set(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    match LANG.load(Ordering::Relaxed) {
        1 => Lang::Es,
        _ => Lang::En,
    }
}

/// Language for a config or environment value: `auto` detects it from the system locale,
/// anything else goes through [`Lang::parse`].
pub fn resolve(value: &str) -> Lang {
    if value.trim().eq_ignore_ascii_case("auto") {
        detect()
    } else {
        Lang::parse(value)
    }
}

/// System locale, by POSIX precedence: the first of `LC_ALL`, `LC_MESSAGES`, `LANG` that is
/// set and not empty. Nothing set (or `C` / `POSIX`) means English.
pub fn detect() -> Lang {
    detect_from(|k| std::env::var(k).ok())
}

fn detect_from(get: impl Fn(&str) -> Option<String>) -> Lang {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| get(k).filter(|v| !v.trim().is_empty()))
        .map(|v| Lang::parse(&v))
        .unwrap_or(Lang::En)
}

/// Sets the language from `PISTASHO_LANG` if present, otherwise from the config value.
pub fn init(config_value: &str) {
    match std::env::var("PISTASHO_LANG") {
        Ok(v) if !v.trim().is_empty() => set(resolve(&v)),
        _ => set(resolve(config_value)),
    }
}

pub fn tr(en: &'static str, es: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Es => es,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_language() {
        assert_eq!(Lang::parse("es"), Lang::Es);
        assert_eq!(Lang::parse(" ES_MX "), Lang::Es);
        assert_eq!(Lang::parse("en"), Lang::En);
        assert_eq!(Lang::parse(""), Lang::En);
        assert_eq!(Lang::parse("fr"), Lang::En);
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn detect_locale() {
        assert_eq!(detect_from(env(&[("LANG", "es_MX.UTF-8")])), Lang::Es);
        assert_eq!(detect_from(env(&[("LANG", "en_US.UTF-8")])), Lang::En);
        // LC_ALL beats LANG, and an empty value is skipped
        assert_eq!(
            detect_from(env(&[("LC_ALL", "es_ES.UTF-8"), ("LANG", "en_US.UTF-8")])),
            Lang::Es
        );
        assert_eq!(
            detect_from(env(&[("LC_ALL", ""), ("LANG", "es_ES.UTF-8")])),
            Lang::Es
        );
        // C / POSIX and nothing set are English
        assert_eq!(detect_from(env(&[("LANG", "C")])), Lang::En);
        assert_eq!(detect_from(env(&[("LANG", "POSIX")])), Lang::En);
        assert_eq!(detect_from(env(&[])), Lang::En);
    }

    #[test]
    fn resolve_values() {
        assert_eq!(resolve("es"), Lang::Es);
        assert_eq!(resolve("en"), Lang::En);
        // `auto` follows the machine, whatever it is: just check it does not panic
        let _ = resolve(" AUTO ");
    }
}
