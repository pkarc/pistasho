//! Command index: tldr (precomputed by `pistasho update`), your cheatsheet and your history.

use crate::config::{self, Config};
use crate::i18n::{lang, tr, Lang};
use crate::{history, placeholders, tldr};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufReader, BufWriter, Read};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Tldr,
    Cheat,
    Hist,
}

impl Source {
    /// Label shown next to each result (translated).
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

#[derive(Debug, Serialize, Deserialize)]
struct IndexFile {
    version: u32,
    languages: Vec<String>,
    entries: Vec<Entry>,
}

const INDEX_VERSION: u32 = 1;

// ---------------------------------------------------------------- tldr

/// Downloads the tldr zip and builds the index.
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
    let path = config::index_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    let file = std::fs::File::create(&path)?;
    serde_json::to_writer(
        BufWriter::new(file),
        &IndexFile {
            version: INDEX_VERSION,
            languages: cfg.languages.clone(),
            entries,
        },
    )?;
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

fn load_tldr() -> Result<Option<Vec<Entry>>> {
    let path = config::index_path()?;
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let idx: IndexFile = serde_json::from_reader(BufReader::new(file)).with_context(|| match lang() {
        Lang::En => format!("corrupt index: {} (run `pistasho update`)", path.display()),
        Lang::Es => format!("índice dañado: {} (ejecuta `pistasho update`)", path.display()),
    })?;
    if idx.version != INDEX_VERSION {
        anyhow::bail!(tr(
            "the index is from another version; run `pistasho update`",
            "el índice es de otra versión; ejecuta `pistasho update`"
        ));
    }
    Ok(Some(idx.entries))
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
        .map(|h| Entry {
            tool: first_word(&h.cmd),
            desc: match (lang(), h.count) {
                (Lang::En, 1) => "used 1 time".into(),
                (Lang::En, n) => format!("used {n} times"),
                (Lang::Es, 1) => "usado 1 vez".into(),
                (Lang::Es, n) => format!("usado {n} veces"),
            },
            alt: String::new(),
            cmd: h.cmd,
            source: Source::Hist,
            origin: tr("history", "historial").into(),
            count: h.count,
        })
        .collect()
}

// ---------------------------------------------------------------- everything together

pub struct Index {
    pub entries: Vec<Entry>,
    pub has_tldr: bool,
}

impl Index {
    pub fn load(cfg: &Config) -> Result<Self> {
        let mut entries = load_cheats()?;
        entries.extend(load_history(cfg));
        let tldr = load_tldr()?;
        let has_tldr = tldr.is_some();
        entries.extend(tldr.unwrap_or_default());
        Ok(Self { entries, has_tldr })
    }
}
