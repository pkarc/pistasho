//! `pistasho ask`: turn a question in natural language into ranked command candidates, using
//! BM25 over the SQLite full-text index (no model, no network).
//!
//! The pipeline: drop stop words from the question, search with stemmed prefix terms, weight the
//! columns, boost your own commands and the ones you use often, and collapse duplicates.

use crate::db::{self, Db, Weights, DEFAULT_WEIGHTS};
use crate::index::{Entry, Source};
use crate::placeholders;
use anyhow::Result;
use std::collections::HashSet;

/// How many BM25 hits are fetched before re-ranking and de-duplicating.
const POOL: usize = 60;

/// Tunable knobs. The values come from `pistasho eval --mode bm25`, see docs/eval.md.
pub struct Params {
    pub weights: Weights,
    /// Multiplier for your own cheats.
    pub cheat_boost: f64,
    /// Each use in your history adds this fraction to the score, up to `hist_boost_cap` uses.
    pub hist_boost_per_use: f64,
    pub hist_boost_cap: u32,
}

pub const PARAMS: Params = Params {
    weights: DEFAULT_WEIGHTS,
    cheat_boost: 1.3,
    hist_boost_per_use: 0.03,
    hist_boost_cap: 10,
};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub entry: Entry,
    pub score: f64,
}

/// Words that say nothing about which command is wanted, Spanish and English. Compared without
/// accents, so `qué` and `que` are both here as `que`.
const STOP_WORDS: &[&str] = &[
    // Spanish
    "a", "al", "algo", "como", "con", "cual", "cuales", "cuando", "cuanto", "de", "del", "desde", "donde",
    "el", "ella", "en", "es", "esta", "estan", "este", "esto", "hay", "la", "las", "le", "les", "lo", "los",
    "me", "mi", "mis", "mas", "muy", "no", "nos", "o", "para", "pero", "por", "que", "quiero", "se", "si",
    "sin", "sobre", "son", "su", "sus", "te", "tengo", "tu", "tus", "un", "una", "uno", "unos", "unas", "y",
    "ya", "yo", "necesito", "puedo", "puede", "hacer", "haz", "dame", "dime", "ver", "veo", "ves", "ese",
    "eso", "todos", "todas", "todo", "esa", "esos", "esas", // English
    "an", "the", "of", "to", "in", "on", "for", "with", "and", "or", "is", "are", "how", "do", "does",
    "what", "which", "i", "my", "you", "it", "this", "that", "from", "by", "at", "as", "be", "can", "want",
    "need", "all",
];

/// Lowercase and drop the accents of Spanish letters, to compare against `STOP_WORDS`.
fn fold(word: &str) -> String {
    word.chars()
        .map(|c| match c {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' | 'ü' => 'u',
            c => c,
        })
        .collect()
}

/// The words of the question that are worth searching for.
pub fn terms(question: &str) -> Vec<String> {
    db::words(question)
        .into_iter()
        .filter(|w| !STOP_WORDS.contains(&fold(w).as_str()))
        .collect()
}

/// Your own commands and the ones you run often should win ties against tldr.
fn boost(e: &Entry, p: &Params) -> f64 {
    match e.source {
        Source::Cheat => p.cheat_boost,
        Source::Hist => 1.0 + e.count.min(p.hist_boost_cap) as f64 * p.hist_boost_per_use,
        Source::Tldr => 1.0,
    }
}

/// The `limit` best candidates for a question, best first.
pub fn ask(db: &Db, question: &str, limit: usize) -> Result<Vec<Candidate>> {
    ask_with(db, question, limit, &PARAMS)
}

pub fn ask_with(db: &Db, question: &str, limit: usize, params: &Params) -> Result<Vec<Candidate>> {
    let words = terms(question);
    let mut ranked: Vec<Candidate> = db
        .fts_search_words(&words, POOL, params.weights)?
        .into_iter()
        .map(|(entry, score)| Candidate {
            score: score * boost(&entry, params),
            entry,
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));

    let mut seen = HashSet::new();
    ranked.retain(|c| seen.insert(placeholders::shape(&c.entry.cmd)));
    ranked.truncate(limit);
    Ok(ranked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool: &str, cmd: &str, desc: &str, alt: &str, source: Source, count: u32) -> Entry {
        Entry {
            cmd: cmd.into(),
            desc: desc.into(),
            alt: alt.into(),
            tool: tool.into(),
            source,
            origin: "test".into(),
            count,
        }
    }

    fn db_with(tldr: Vec<Entry>, cheats: Vec<Entry>, hist: Vec<Entry>) -> Db {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(Source::Tldr, &tldr, &[]).unwrap();
        db.replace_source(Source::Cheat, &cheats, &[]).unwrap();
        db.replace_source(Source::Hist, &hist, &[]).unwrap();
        db
    }

    #[test]
    fn removes_stop_words_in_both_languages() {
        assert_eq!(terms("¿Cómo puedo ver mi IP?"), vec!["ip"]);
        assert_eq!(
            terms("qué proceso usa el puerto 8080"),
            vec!["proceso", "usa", "puerto", "8080"]
        );
        assert_eq!(terms("how do I compress the folder"), vec!["compress", "folder"]);
        assert_eq!(terms("de la en"), Vec::<String>::new());
        // accents do not matter when comparing against the list
        assert_eq!(terms("más rápido"), vec!["rápido"]);
    }

    #[test]
    fn question_made_only_of_stop_words_finds_nothing() {
        let db = db_with(
            vec![entry("ip", "ip a", "Show addresses", "", Source::Tldr, 0)],
            vec![],
            vec![],
        );
        assert!(ask(&db, "qué es eso", 5).unwrap().is_empty());
    }

    #[test]
    fn dedupes_translated_placeholders() {
        let db = db_with(
            vec![
                entry(
                    "tar",
                    "tar czf {{path/to/file}}.tar.gz",
                    "create a gzipped archive",
                    "",
                    Source::Tldr,
                    0,
                ),
                entry(
                    "tar",
                    "tar czf {{ruta/al/archivo}}.tar.gz",
                    "create a gzipped archive",
                    "",
                    Source::Tldr,
                    0,
                ),
            ],
            vec![],
            vec![],
        );
        assert_eq!(ask(&db, "gzipped archive", 5).unwrap().len(), 1);
    }

    #[test]
    fn your_cheat_beats_an_equal_tldr_entry() {
        let db = db_with(
            vec![entry(
                "fuser",
                "fuser -k {{path}}",
                "kill the process on a port",
                "",
                Source::Tldr,
                0,
            )],
            vec![entry(
                "fuser",
                "fuser -k {{port}}/tcp",
                "kill the process on a port",
                "",
                Source::Cheat,
                0,
            )],
            vec![],
        );
        let top = ask(&db, "kill the process on a port", 5).unwrap();
        assert_eq!(top[0].entry.source, Source::Cheat);
        assert_eq!(top.len(), 2);
    }

    #[test]
    fn history_matches_by_command_words_and_frequent_ones_rise() {
        let db = db_with(
            vec![],
            vec![],
            vec![
                entry("ssh", "ssh root@10.0.0.12", "", "", Source::Hist, 1),
                entry("ssh", "ssh root@10.0.0.99", "", "", Source::Hist, 9),
                entry("ls", "ls -la /tmp", "", "", Source::Hist, 3),
            ],
        );
        // "root" only occurs in the command text, which is why it is indexed
        let top = ask(&db, "conectar como root", 5).unwrap();
        assert_eq!(top.len(), 2);
        assert_eq!(
            top[0].entry.cmd, "ssh root@10.0.0.99",
            "the more used one comes first"
        );
    }

    #[test]
    fn returns_at_most_the_limit_best_first() {
        let tldr = (0..20)
            .map(|i| {
                entry(
                    "t",
                    &format!("tool{i} --compress"),
                    &format!("compress thing number {i}"),
                    "",
                    Source::Tldr,
                    0,
                )
            })
            .collect();
        let db = db_with(tldr, vec![], vec![]);
        let top = ask(&db, "compress", 5).unwrap();
        assert_eq!(top.len(), 5);
        assert!(top.windows(2).all(|w| w[0].score >= w[1].score));
    }
}
