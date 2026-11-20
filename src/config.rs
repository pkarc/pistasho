//! Configuración en `~/.config/pistasho/config.toml` y asistente `pistasho config`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

pub const TLDR_URL: &str = "https://github.com/tldr-pages/tldr/releases/latest/download/tldr.zip";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Idiomas de tldr en orden de preferencia.
    pub languages: Vec<String>,
    /// Plataformas de tldr a indexar.
    pub platforms: Vec<String>,
    pub history: bool,
    /// Ruta del historial; por defecto `$HISTFILE` o `~/.zsh_history`.
    pub history_file: Option<String>,
    pub history_limit: usize,
    pub tldr_url: String,
    pub keys: Keys,
    /// Modelo de decisión (fase 3). Aún no se usa.
    pub decide: Decide,
    /// LLM de respaldo (fase 4). Aún no se usa.
    pub llm: Llm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Keys {
    /// Buscador (zsh, notación de bindkey).
    pub search: String,
    /// Modo pregunta (fase 2).
    pub ask: String,
    /// Tecla tras el prefijo de tmux para el popup.
    pub tmux: String,
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
    /// `none`, `anthropic` u `ollama`.
    pub provider: String,
    pub model: Option<String>,
    pub endpoint: Option<String>,
    pub api_key_env: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            languages: vec!["es".into(), "en".into()],
            platforms: vec!["common".into(), "linux".into()],
            history: true,
            history_file: None,
            history_limit: 5000,
            tldr_url: TLDR_URL.into(),
            keys: Keys::default(),
            decide: Decide::default(),
            llm: Llm::default(),
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            search: "^G".into(),
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
        .context("no encontré el directorio de configuración")?
        .join("pistasho"))
}

pub fn data_dir() -> Result<PathBuf> {
    Ok(dirs::data_dir()
        .context("no encontré el directorio de datos")?
        .join("pistasho"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

pub fn cheats_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("cheats.toml"))
}

pub fn index_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("index.json"))
}

impl Config {
    /// Carga la configuración; si no existe, usa los valores por defecto sin preguntar
    /// (se llama desde el widget y desde `init`, que deben ser silenciosos).
    pub fn load() -> Result<Self> {
        let path = config_path()?;
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("error en {}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("no pude leer {}", path.display())),
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
            return Some(PathBuf::from(p));
        }
        dirs::home_dir().map(|h| h.join(".zsh_history"))
    }
}

pub fn expand_home(p: &str) -> PathBuf {
    match (p.strip_prefix("~/"), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ => PathBuf::from(p),
    }
}

// ---------------------------------------------------------------- asistente

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
        let v = ask(stdin, "Opción", &default.to_string())?;
        match v.parse::<usize>() {
            Ok(n) if (1..=options.len()).contains(&n) => return Ok(options[n - 1].0.to_string()),
            _ => println!("  Elige un número entre 1 y {}", options.len()),
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

/// `pistasho config`: pregunta lo esencial y guarda el archivo.
/// Devuelve `true` si el usuario quiere descargar tldr ahora.
pub fn wizard() -> Result<(Config, bool)> {
    let mut cfg = Config::load()?;
    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    println!("pistasho · configuración (Enter acepta el valor entre corchetes)\n");

    let langs = ask(
        &mut stdin,
        "Idiomas de tldr, en orden de preferencia",
        &cfg.languages.join(","),
    )?;
    cfg.languages = langs
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let hist = ask(
        &mut stdin,
        "¿Incluir tu historial de zsh en las búsquedas? (s/n)",
        if cfg.history { "s" } else { "n" },
    )?;
    cfg.history = !hist.to_lowercase().starts_with('n');

    cfg.keys.search = ask(
        &mut stdin,
        "Atajo del buscador en zsh (notación bindkey)",
        &cfg.keys.search,
    )?;
    cfg.keys.tmux = ask(
        &mut stdin,
        "Tecla del popup tras el prefijo de tmux",
        &cfg.keys.tmux,
    )?;

    cfg.decide.provider = choose(
        &mut stdin,
        "Modelo de decisión (elige entre candidatos locales; fase 3):",
        &[
            ("none", "solo búsqueda local"),
            ("laya", "Laya local en ONNX (~0.5–1.7 GB de RAM)"),
            ("laya-serve", "servidor laya-serve en tu red"),
            ("jev", "API de Jev (TypeSafe)"),
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
            cfg.decide.endpoint = opt(ask(&mut stdin, "URL de laya-serve", &d)?);
        }
        "jev" => {
            let d = cfg
                .decide
                .api_key_env
                .clone()
                .unwrap_or_else(|| "JEV_API_KEY".into());
            cfg.decide.api_key_env = opt(ask(&mut stdin, "Variable de entorno con la API key", &d)?);
        }
        _ => {}
    }

    cfg.llm.provider = choose(
        &mut stdin,
        "LLM de respaldo (genera comandos que no están en local; fase 4):",
        &[
            ("none", "sin LLM, todo offline"),
            ("anthropic", "API de Anthropic"),
            ("ollama", "Ollama local o en tu red"),
        ],
        &cfg.llm.provider.clone(),
    )?;
    match cfg.llm.provider.as_str() {
        "anthropic" => {
            let m = cfg.llm.model.clone().unwrap_or_else(|| "claude-haiku-5-5".into());
            cfg.llm.model = opt(ask(&mut stdin, "Modelo", &m)?);
            let k = cfg
                .llm
                .api_key_env
                .clone()
                .unwrap_or_else(|| "ANTHROPIC_API_KEY".into());
            cfg.llm.api_key_env = opt(ask(&mut stdin, "Variable de entorno con la API key", &k)?);
        }
        "ollama" => {
            let e = cfg
                .llm
                .endpoint
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".into());
            cfg.llm.endpoint = opt(ask(&mut stdin, "URL de Ollama", &e)?);
            let m = cfg.llm.model.clone().unwrap_or_else(|| "qwen2.5-coder:7b".into());
            cfg.llm.model = opt(ask(&mut stdin, "Modelo", &m)?);
        }
        _ => {}
    }

    let path = cfg.save()?;
    println!("\n✓ Guardado en {}", path.display());

    let dl = ask(&mut stdin, "\n¿Descargar las páginas de tldr ahora? (s/n)", "s")?;
    Ok((cfg, !dl.to_lowercase().starts_with('n')))
}
