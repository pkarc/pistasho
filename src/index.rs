//! Command index: tldr (stored by `pistasho update`), your cheatsheet and your history, all kept
//! in the SQLite database (`db.rs`). Cheats and history are re-synced when their files change.

use crate::config::{self, Config};
use crate::db::Db;
use crate::i18n::{lang, tr, Lang};
use crate::{history, placeholders, tldr};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Tldr,
    Cheat,
    Hist,
}

impl Source {
    /// Label shown next to each result (translated).
    /// Name stored in the database.
    pub fn db_name(self) -> &'static str {
        match self {
            Source::Tldr => "tldr",
            Source::Cheat => "cheat",
            Source::Hist => "hist",
        }
    }

    pub fn from_db_name(s: &str) -> Option<Self> {
        match s {
            "tldr" => Some(Source::Tldr),
            "cheat" => Some(Source::Cheat),
            "hist" => Some(Source::Hist),
            _ => None,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Source::Tldr => "tldr",
            Source::Cheat => tr("mine", "mío"),
            Source::Hist => "hist",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub cmd: String,
    pub desc: String,
    /// Extra text used only for searching: descriptions in other languages, page summary, tags.
    #[serde(default)]
    pub alt: String,
    /// Tool name (tldr page or first word of the command).
    pub tool: String,
    pub source: Source,
    /// Where it comes from, for display: `tldr · linux/ss`, `cheats.toml`, `history`.
    pub origin: String,
    /// Times used (history).
    #[serde(default)]
    pub count: u32,
}

// ---------------------------------------------------------------- tldr

/// Downloads the tldr zip and stores its entries in the database.
pub fn update(cfg: &Config) -> Result<(usize, usize)> {
    eprintln!("{} {} …", tr("Downloading", "Descargando"), cfg.tldr_url);
    let resp = ureq::get(&cfg.tldr_url)
        .call()
        .context(tr("tldr download failed", "falló la descarga de tldr"))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .take(300 * 1024 * 1024)
        .read_to_end(&mut bytes)?;
    eprintln!(
        "{} {:.1} MB …",
        tr("Processing", "Procesando"),
        bytes.len() as f64 / 1e6
    );

    let (entries, pages) = build_from_zip(&bytes, &cfg.languages, &cfg.platforms)?;
    let n = entries.len();
    let mut db = Db::open(&config::db_path()?)?;
    let updated = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
        .to_string();
    db.replace_source(
        Source::Tldr,
        &entries,
        &[
            ("languages", &cfg.languages.join(",")),
            ("tldr_updated", &updated),
        ],
    )?;
    // the JSON index of earlier versions is no longer used
    if let Ok(dir) = config::data_dir() {
        let _ = std::fs::remove_file(dir.join("index.json"));
    }
    Ok((pages, n))
}

pub fn build_from_zip(bytes: &[u8], langs: &[String], platforms: &[String]) -> Result<(Vec<Entry>, usize)> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .context(tr("invalid tldr zip", "zip de tldr inválido"))?;
    // (platform, page) -> language -> page
    let mut pages: HashMap<(String, String), HashMap<String, tldr::Page>> = HashMap::new();

    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        if file.is_dir() {
            continue;
        }
        let Some(pp) = tldr::page_path(file.name()) else {
            continue;
        };
        if !langs.contains(&pp.lang) || !platforms.contains(&pp.platform) {
            continue;
        }
        let mut md = String::new();
        if file.read_to_string(&mut md).is_err() {
            continue;
        }
        pages
            .entry((pp.platform, pp.name))
            .or_default()
            .insert(pp.lang, tldr::parse(&md));
    }

    let n_pages = pages.len();
    let mut entries = Vec::new();
    for ((platform, name), by_lang) in pages {
        entries.extend(merge_page(&platform, &name, &by_lang, langs));
    }
    entries.sort_by(|a, b| a.tool.cmp(&b.tool));
    Ok((entries, n_pages))
}

/// Joins the translations of a page: the main description comes from the preferred language
/// and the rest go into `alt` so a search in any language finds it.
fn merge_page(
    platform: &str,
    name: &str,
    by_lang: &HashMap<String, tldr::Page>,
    langs: &[String],
) -> Vec<Entry> {
    let ordered: Vec<&tldr::Page> = langs.iter().filter_map(|l| by_lang.get(l)).collect();
    let summaries: Vec<&str> = ordered.iter().map(|p| p.summary.as_str()).collect();

    // normalized command -> descriptions in language order
    let mut order: Vec<String> = Vec::new();
    let mut descs: HashMap<String, Vec<String>> = HashMap::new();
    for page in &ordered {
        for ex in &page.examples {
            let cmd = placeholders::normalize_tldr(&ex.cmd);
            let list = descs.entry(cmd.clone()).or_insert_with(|| {
                order.push(cmd.clone());
                Vec::new()
            });
            list.push(ex.desc.clone());
        }
    }

    order
        .into_iter()
        .map(|cmd| {
            let d = descs.remove(&cmd).unwrap_or_default();
            let desc = d.first().cloned().unwrap_or_default();
            let mut alt: Vec<&str> = d.iter().skip(1).map(String::as_str).collect();
            alt.extend(summaries.iter().copied());
            Entry {
                cmd,
                desc,
                alt: alt.join(" · "),
                tool: name.to_string(),
                source: Source::Tldr,
                origin: format!("tldr · {platform}/{name}"),
                count: 0,
            }
        })
        .collect()
}

// ---------------------------------------------------------------- cheatsheet

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CheatFile {
    #[serde(default, rename = "cheat")]
    pub cheats: Vec<Cheat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Cheat {
    pub cmd: String,
    pub desc: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

fn first_word(cmd: &str) -> String {
    let mut words = cmd.split_whitespace();
    let mut w = words.next().unwrap_or("");
    if w == "sudo" {
        w = words.next().unwrap_or(w);
    }
    w.to_string()
}

pub fn load_cheats() -> Result<Vec<Entry>> {
    let path = config::cheats_path()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let file: CheatFile = toml::from_str(&text)
        .with_context(|| format!("{} {}", tr("error in", "error en"), path.display()))?;
    Ok(file
        .cheats
        .into_iter()
        .map(|c| Entry {
            tool: first_word(&c.cmd),
            alt: c.tags.join(" "),
            cmd: c.cmd,
            desc: c.desc,
            source: Source::Cheat,
            origin: "cheats.toml".into(),
            count: 0,
        })
        .collect())
}

/// Appends an entry to the end of `cheats.toml` without rewriting what you already have.
pub fn add_cheat(cheat: Cheat) -> Result<std::path::PathBuf> {
    let path = config::cheats_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let block = toml::to_string(&CheatFile { cheats: vec![cheat] })?;
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    if !existing.is_empty() && !existing.ends_with("\n\n") {
        existing.push('\n');
    }
    existing.push_str(&block);
    std::fs::write(&path, existing)?;
    Ok(path)
}

// ---------------------------------------------------------------- history

pub fn load_history(cfg: &Config) -> Vec<Entry> {
    if !cfg.history {
        return Vec::new();
    }
    let Some(path) = cfg.history_path() else {
        return Vec::new();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Vec::new();
    };
    history::load(&bytes, cfg.history_limit)
        .into_iter()
        .map(|h| {
            let (desc, origin) = history_labels(h.count);
            Entry {
                tool: first_word(&h.cmd),
                desc,
                alt: String::new(),
                cmd: h.cmd,
                source: Source::Hist,
                origin,
                count: h.count,
            }
        })
        .collect()
}

/// Description and origin of a history entry. They depend on the UI language, so they are
/// recomputed on every load instead of trusting what the database stored.
fn history_labels(count: u32) -> (String, String) {
    let desc = match (lang(), count) {
        (Lang::En, 1) => "used 1 time".into(),
        (Lang::En, n) => format!("used {n} times"),
        (Lang::Es, 1) => "usado 1 vez".into(),
        (Lang::Es, n) => format!("usado {n} veces"),
    };
    (desc, tr("history", "historial").into())
}

// ---------------------------------------------------------------- everything together

pub struct Index {
    pub entries: Vec<Entry>,
    pub has_tldr: bool,
}

impl Index {
    pub fn load(cfg: &Config) -> Result<Self> {
        let mut db = Db::open(&config::db_path()?)?;
        sync(&mut db, cfg)?;
        let mut entries = db.load_all()?;
        for e in entries.iter_mut().filter(|e| e.source == Source::Hist) {
            (e.desc, e.origin) = history_labels(e.count);
        }
        let has_tldr = db.count_source(Source::Tldr)? > 0;
        Ok(Self { entries, has_tldr })
    }
}

/// `mtime-in-nanoseconds:length` of a file, or `-` if it does not exist.
/// It changes whenever the file does.
fn fingerprint(path: Option<&std::path::Path>) -> String {
    let Some(meta) = path.and_then(|p| std::fs::metadata(p).ok()) else {
        return "-".into();
    };
    let nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    format!("{nanos}:{}", meta.len())
}

/// Brings the cheats and history rows up to date, but only when their files changed since the
/// last sync (compared by fingerprint, stored in `meta`), so a normal open reads nothing.
fn sync(db: &mut Db, cfg: &Config) -> Result<()> {
    let cheats_fp = fingerprint(Some(&config::cheats_path()?));
    if db.meta("cheats_fp")?.as_deref() != Some(cheats_fp.as_str()) {
        db.replace_source(Source::Cheat, &load_cheats()?, &[("cheats_fp", &cheats_fp)])?;
    }

    let hist_path = cfg.history_path();
    let history_fp = format!(
        "{}|{}|{}|{}",
        cfg.history,
        cfg.history_limit,
        hist_path
            .as_ref()
            .map_or(String::new(), |p| p.display().to_string()),
        fingerprint(hist_path.as_deref())
    );
    if db.meta("history_fp")?.as_deref() != Some(history_fp.as_str()) {
        db.replace_source(Source::Hist, &load_history(cfg), &[("history_fp", &history_fp)])?;
    }
    Ok(())
}
