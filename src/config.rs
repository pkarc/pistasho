//! Configuration in `~/.config/pistasho/config.toml` and the `pistasho config` wizard.

use crate::i18n::{self, tr};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub const TLDR_URL: &str = "https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Interface language: `en` (default), `es`, or `auto` to detect it from the system locale.
    /// `PISTASHO_LANG` overrides it.
    pub language: String,
    /// tldr languages in order of preference; the first one provides the displayed description.
    pub languages: Vec<String>,
    /// tldr platforms to index.
    pub platforms: Vec<String>,
    pub history: bool,
    /// History file path; defaults to `$HISTFILE` or `~/.zsh_history`.
    pub history_file: Option<String>,
    pub history_limit: usize,
    pub tldr_url: String,
    pub keys: Keys,
    pub ui: Ui,
    pub install: Install,
    pub search: Search,
    /// Decision model (phase 3). Not used yet.
    pub decide: Decide,
    /// LLM fallback (phase 4). Not used yet.
    pub llm: Llm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    /// The one zsh shortcut (bindkey notation; `^ ` is Ctrl-Space). Config files from before
    /// ADR 0001 may still have a `search` key; it is ignored.
    pub ask: String,
    /// Key after the tmux prefix for the popup.
    pub tmux: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    /// Searcher height in the zsh widget: `40%`, a number of lines (`15`) or `full` for the
    /// whole screen. The tmux popup always uses its own full area.
    pub height: String,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            height: DEFAULT_HEIGHT.into(),
        }
    }
}

/// When a command needs a program that is not installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Install {
    /// Offer to install it (with the exact command shown and an explicit key to confirm).
    /// There is no setting that installs without asking.
    pub offer: bool,
}

/// How `ask` ranks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Search {
    /// Add multilingual embeddings to BM25 (`pistasho update` downloads a ~135 MB model and
    /// embeds the entries). Without the model, or when false, `ask` is BM25 only.
    /// `PISTASHO_EMBEDDINGS=0|1` overrides it for one command.
    pub embeddings: bool,
}

impl Default for Search {
    fn default() -> Self {
        Self { embeddings: true }
    }
}

impl Default for Install {
    fn default() -> Self {
        Self { offer: true }
    }
}

pub const DEFAULT_HEIGHT: &str = "40%";
/// The searcher layout needs this many rows to be usable.
pub const MIN_INLINE_ROWS: u16 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Height {
    Full,
    Lines(u16),
    Percent(u16),
}

impl Height {
    /// `full`, `40%` or `15`. Anything else is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        if s == "full" {
            return Some(Height::Full);
        }
        if let Some(p) = s.strip_suffix('%') {
            return p
                .trim()
                .parse::<u16>()
                .ok()
                .filter(|p| (1..=100).contains(p))
                .map(Height::Percent);
        }
        s.parse::<u16>().ok().filter(|n| *n > 0).map(Height::Lines)
    }

    /// Rows for an inline viewport on a terminal with `term_rows` rows; `None` = full screen.
    pub fn rows(self, term_rows: u16) -> Option<u16> {
        let wanted = match self {
            Height::Full => return None,
            Height::Lines(n) => n,
            Height::Percent(p) => (term_rows as u32 * p as u32 / 100) as u16,
        };
        // leave the prompt line visible above the viewport
        let max = term_rows.saturating_sub(1).max(1);
        Some(wanted.max(MIN_INLINE_ROWS).min(max))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Decide {
    /// `none`, `laya`, `laya-serve` o `jev`.
    pub provider: String,
    pub endpoint: Option<String>,
    pub api_key_env: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Llm {
    /// `none`, `anthropic` or `ollama`.
    pub provider: String,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub api_key_env: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            language: "en".into(),
            languages: vec!["en".into(), "es".into()],
            platforms: vec!["common".into(), "linux".into()],
            history: true,
            history_file: None,
            history_limit: 5000,
            tldr_url: TLDR_URL.into(),
            keys: Keys::default(),
            ui: Ui::default(),
            install: Install::default(),
            search: Search::default(),
            decide: Decide::default(),
            llm: Llm::default(),
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            ask: "^ ".into(),
            tmux: "g".into(),
        }
    }
}

impl Default for Decide {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            endpoint: None,
            api_key_env: None,
        }
    }
}

impl Default for Llm {
    fn default() -> Self {
        Self {
            provider: "none".into(),
            model: None,
            endpoint: None,
            api_key_env: None,
        }
    }
}

pub fn config_dir() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context(tr(
            "could not find the config directory",
            "no encontré el directorio de configuración",
        ))?
        .join("pistasho"))
}

pub fn data_dir() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context(tr(
            "could not find the data directory",
            "no encontré el directorio de datos",
        ))?
        .join("pistasho"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn cheats_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cheats.toml"))
}

pub fn db_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("pistasho.db"))
}

impl Config {
    /// Loads the configuration; if it does not exist, uses the defaults without asking
    /// (called from the widget and from `init`, which must be silent).
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s)
                .with_context(|| format!("{} {}", tr("error in", "error en"), path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => {
                Err(e).with_context(|| format!("{} {}", tr("could not read", "no pude leer"), path.display()))
            }
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(path)
    }

    pub fn history_path(&self) -> Option<PathBuf> {
        if let Some(p) = &self.history_file {
            return Some(expand_home(p));
        }
        if let Ok(p) = std::env::var("HISTFILE") {
            if !p.is_empty() {
                return Some(expand_home(&p));
            }
        }
        // zsh keeps HISTFILE as a plain shell variable, so a child process rarely sees it
        // (the widget passes it along, but the tmux popup does not): try the usual places
        dirs::home_dir().map(|h| history_fallback(&h, |p| p.exists()))
    }
}

/// First existing history file among the places zsh setups usually keep it, or
/// `~/.zsh_history` when none exists yet.
fn history_fallback(home: &std::path::Path, exists: impl Fn(&std::path::Path) -> bool) -> PathBuf {
    const CANDIDATES: [&str; 4] = [
        ".zsh_history",
        ".histfile",
        ".local/share/zsh/history",
        ".cache/zsh/history",
    ];
    CANDIDATES
        .iter()
        .map(|c| home.join(c))
        .find(|p| exists(p))
        .unwrap_or_else(|| home.join(CANDIDATES[0]))
}

pub fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(p),
    }
}

// ---------------------------------------------------------------- wizard

fn ask(stdin: &mut impl BufRead, label: &str, default: &str) -> Result<String> {
    print!("{label} [{default}]: ");
    io::stdout().flush()?;
    let mut line = String::new();
    stdin.read_line(&mut line)?;
    let v = line.trim();
    Ok(if v.is_empty() {
        default.to_string()
    } else {
        v.to_string()
    })
}

fn choose(stdin: &mut impl BufRead, label: &str, options: &[(&str, &str)], current: &str) -> Result<String> {
    println!("\n{label}");
    let mut default = 1;
    for (i, (key, desc)) in options.iter().enumerate() {
        if *key == current {
            default = i + 1;
        }
        println!("  {}) {:<11} {}", i + 1, key, desc);
    }
    loop {
        let v = ask(stdin, tr("Option", "Opción"), &default.to_string())?;
        match v.parse::<usize>() {
            Ok(n) if (1..=options.len()).contains(&n) => return Ok(options[n - 1].0.to_string()),
            _ => println!(
                "  {} {}",
                tr("Pick a number between 1 and", "Elige un número entre 1 y"),
                options.len()
            ),
        }
    }
}

fn opt(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// `pistasho config`: asks the essentials and saves the file.
/// Returns `true` if the user wants to download tldr now.
pub fn wizard() -> Result<(Config, bool)> {
    let mut cfg = Config::load()?;
    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    // asked first, in both languages, so everything after it is in the chosen language
    cfg.language = choose(
        &mut stdin,
        "Interface language / Idioma de la interfaz:",
        &[
            ("en", "English"),
            ("es", "Español"),
            ("auto", "detect from the system / detectar del sistema"),
        ],
        &cfg.language.clone(),
    )?;
    i18n::set(i18n::resolve(&cfg.language));
    let yes = tr("y", "s");

    println!(
        "\npistasho · {}\n",
        tr(
            "configuration (Enter accepts the value in brackets)",
            "configuración (Enter acepta el valor entre corchetes)"
        )
    );

    let langs = ask(
        &mut stdin,
        tr(
            "tldr languages, in order of preference",
            "Idiomas de tldr, en orden de preferencia",
        ),
        &cfg.languages.join(","),
    )?;
    cfg.languages = langs
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let hist = ask(
        &mut stdin,
        tr(
            "Include your zsh history in searches? (y/n)",
            "¿Incluir tu historial de zsh en las búsquedas? (s/n)",
        ),
        if cfg.history { yes } else { "n" },
    )?;
    cfg.history = !hist.to_lowercase().starts_with('n');

    cfg.keys.ask = ask(
        &mut stdin,
        tr(
            "Shortcut in zsh (bindkey notation; \"^ \" is Ctrl-Space)",
            "Atajo en zsh (notación bindkey; \"^ \" es Ctrl-Space)",
        ),
        &cfg.keys.ask,
    )?;
    cfg.keys.tmux = ask(
        &mut stdin,
        tr(
            "Popup key after the tmux prefix",
            "Tecla del popup tras el prefijo de tmux",
        ),
        &cfg.keys.tmux,
    )?;

    let h = ask(
        &mut stdin,
        tr(
            "Searcher height in zsh (e.g. 40%, 15 lines, or full)",
            "Alto del buscador en zsh (p. ej. 40%, 15 líneas o full)",
        ),
        &cfg.ui.height,
    )?;
    cfg.ui.height = if Height::parse(&h).is_some() {
        h
    } else {
        DEFAULT_HEIGHT.into()
    };

    let offer = ask(
        &mut stdin,
        tr(
            "Offer to install programs that a command needs and you do not have? (y/n; it always asks first)",
            "¿Ofrecer instalar los programas que un comando necesita y no tienes? (s/n; siempre pregunta antes)",
        ),
        if cfg.install.offer { yes } else { "n" },
    )?;
    cfg.install.offer = !offer.to_lowercase().starts_with('n');

    let emb = ask(
        &mut stdin,
        tr(
            "Understand questions by meaning with a local model? Downloads ~135 MB on update and uses ~600 MB of RAM for about a second per question (y/n)",
            "¿Entender las preguntas por significado con un modelo local? Descarga ~135 MB en update y usa ~600 MB de RAM cerca de un segundo por pregunta (s/n)",
        ),
        if cfg.search.embeddings { yes } else { "n" },
    )?;
    cfg.search.embeddings = !emb.to_lowercase().starts_with('n');

    cfg.decide.provider = choose(
        &mut stdin,
        tr(
            "Decision model (chooses among local candidates; phase 3):",
            "Modelo de decisión (elige entre candidatos locales; fase 3):",
        ),
        &[
            ("none", tr("local search only", "solo búsqueda local")),
            (
                "laya",
                tr(
                    "local Laya on ONNX (~0.5–1.7 GB of RAM)",
                    "Laya local en ONNX (~0.5–1.7 GB de RAM)",
                ),
            ),
            (
                "laya-serve",
                tr(
                    "laya-serve server on your network",
                    "servidor laya-serve en tu red",
                ),
            ),
            ("jev", tr("Jev API (TypeSafe)", "API de Jev (TypeSafe)")),
        ],
        &cfg.decide.provider.clone(),
    )?;
    match cfg.decide.provider.as_str() {
        "laya-serve" => {
            let d = cfg
                .decide
                .endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:8000".into());
            cfg.decide.endpoint = opt(ask(&mut stdin, tr("laya-serve URL", "URL de laya-serve"), &d)?);
        }
        "jev" => {
            let d = cfg
                .decide
                .api_key_env
                .clone()
                .unwrap_or_else(|| "JEV_API_KEY".into());
            cfg.decide.api_key_env = opt(ask(
                &mut stdin,
                tr(
                    "Environment variable holding the API key",
                    "Variable de entorno con la API key",
                ),
                &d,
            )?);
        }
        _ => {}
    }

    cfg.llm.provider = choose(
        &mut stdin,
        tr(
            "LLM fallback (generates commands that are not available locally; phase 4):",
            "LLM de respaldo (genera comandos que no están en local; fase 4):",
        ),
        &[
            ("none", tr("no LLM, fully offline", "sin LLM, todo offline")),
            ("anthropic", tr("Anthropic API", "API de Anthropic")),
            (
                "ollama",
                tr("Ollama, local or on your network", "Ollama local o en tu red"),
            ),
        ],
        &cfg.llm.provider.clone(),
    )?;
    match cfg.llm.provider.as_str() {
        "anthropic" => {
            let m = cfg.llm.model.clone().unwrap_or_else(|| "claude-haiku-5-5".into());
            cfg.llm.model = opt(ask(&mut stdin, tr("Model", "Modelo"), &m)?);
            let k = cfg
                .llm
                .api_key_env
                .clone()
                .unwrap_or_else(|| "ANTHROPIC_API_KEY".into());
            cfg.llm.api_key_env = opt(ask(
                &mut stdin,
                tr(
                    "Environment variable holding the API key",
                    "Variable de entorno con la API key",
                ),
                &k,
            )?);
        }
        "ollama" => {
            let e = cfg
                .llm
                .endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".into());
            cfg.llm.endpoint = opt(ask(&mut stdin, tr("Ollama URL", "URL de Ollama"), &e)?);
            let m = cfg.llm.model.clone().unwrap_or_else(|| "qwen2.5-coder:7b".into());
            cfg.llm.model = opt(ask(&mut stdin, tr("Model", "Modelo"), &m)?);
        }
        _ => {}
    }

    let path = cfg.save()?;
    println!("\n✓ {} {}", tr("Saved to", "Guardado en"), path.display());

    let dl = ask(
        &mut stdin,
        tr(
            "\nDownload the tldr pages now? (y/n)",
            "\n¿Descargar las páginas de tldr ahora? (s/n)",
        ),
        yes,
    )?;
    Ok((cfg, !dl.to_lowercase().starts_with('n')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_history_in_usual_places() {
        let home = std::path::Path::new("/home/u");
        let only = |name: &'static str| move |p: &std::path::Path| p == home.join(name);
        assert_eq!(history_fallback(home, only(".histfile")), home.join(".histfile"));
        assert_eq!(
            history_fallback(home, only(".local/share/zsh/history")),
            home.join(".local/share/zsh/history")
        );
        // ~/.zsh_history wins when several exist, and is the answer when none does
        assert_eq!(history_fallback(home, |_| true), home.join(".zsh_history"));
        assert_eq!(history_fallback(home, |_| false), home.join(".zsh_history"));
    }

    #[test]
    fn parse_height() {
        assert_eq!(Height::parse("40%"), Some(Height::Percent(40)));
        assert_eq!(Height::parse(" 15 "), Some(Height::Lines(15)));
        assert_eq!(Height::parse("FULL"), Some(Height::Full));
        assert_eq!(Height::parse("0%"), None);
        assert_eq!(Height::parse("150%"), None);
        assert_eq!(Height::parse("0"), None);
        assert_eq!(Height::parse("tall"), None);
    }

    #[test]
    fn height_rows() {
        assert_eq!(Height::Full.rows(50), None);
        assert_eq!(Height::Percent(40).rows(50), Some(20));
        // never below the minimum usable layout
        assert_eq!(Height::Percent(10).rows(50), Some(MIN_INLINE_ROWS));
        // never the whole terminal: the prompt line stays visible
        assert_eq!(Height::Percent(100).rows(30), Some(29));
        assert_eq!(Height::Lines(5).rows(8), Some(7));
    }
}
