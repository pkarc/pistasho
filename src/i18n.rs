//! Interface language. English is the default; Spanish is selected with `language = "es"`
//! in the config file or the `PISTASHO_LANG` environment variable (the variable wins).
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

/// Sets the language from `PISTASHO_LANG` if present, otherwise from the config value.
pub fn init(config_value: &str) {
    match std::env::var("PISTASHO_LANG") {
        Ok(v) if !v.trim().is_empty() => set(Lang::parse(&v)),
        _ => set(Lang::parse(config_value)),
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
}
