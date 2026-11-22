//! `pistasho ask`: turn a question in natural language into ranked command candidates, using
//! BM25 over the SQLite full-text index (no model, no network).
//!
//! The pipeline: drop stop words from the question, search with stemmed prefix terms, weight the
//! columns, boost your own commands and the ones you use often, and collapse duplicates. When the
//! embedding model is available, a second ranking by meaning is fused with it (RRF).

use crate::db::{self, Db, Weights, DEFAULT_WEIGHTS};
use crate::embed;
use crate::index::{Entry, Source};
use crate::packages;
use crate::placeholders;
use crate::semantic;
use crate::system;
use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

/// How many BM25 hits are fetched before re-ranking and de-duplicating.
const POOL: usize = 60;
/// Nearest vectors fetched before de-duplicating (translations share a shape, so many collapse).
const VECTOR_POOL: usize = 200;
/// If this few entries lack a vector (your cheats after editing `cheats.toml`) they are
/// embedded on the spot; more than that waits for `pistasho update`.
const EMBED_ON_THE_FLY: usize = 200;

/// Tunable knobs. The values come from `pistasho eval --mode bm25`, see docs/eval.md.
/// How the two rankings are fused (reciprocal rank fusion).
#[derive(Debug, Clone, Copy)]
pub struct Fusion {
    /// Weight of the BM25 ranking.
    pub bm25: f64,
    /// Weight of the ranking by meaning.
    pub meaning: f64,
    /// RRF constant: the larger, the flatter the contribution of the top ranks.
    pub k: f64,
}

pub struct Params {
    pub weights: Weights,
    pub fusion: Fusion,
    /// After this many entries of one tool, the next ones of that tool go behind the other tools'
    /// (they are not dropped). 0 = off.
    pub per_tool: usize,
    /// Search the synonyms of the question words too.
    pub synonyms: bool,
    /// Multiplier for your own cheats.
    pub cheat_boost: f64,
    /// Each use in your history adds this fraction to the score, up to `hist_boost_cap` uses.
    pub hist_boost_per_use: f64,
    pub hist_boost_cap: u32,
}

pub const PARAMS: Params = Params {
    weights: DEFAULT_WEIGHTS,
    fusion: Fusion {
        bm25: 1.0,
        meaning: 2.0,
        k: 10.0,
    },
    per_tool: 1,
    synonyms: true,
    cheat_boost: 1.3,
    hist_boost_per_use: 0.03,
    hist_boost_cap: 10,
};

#[derive(Debug, Clone)]
pub struct Candidate {
    pub entry: Entry,
    pub score: f64,
    /// Programs the command runs that are not installed (shell builtins do not count).
    /// Empty means the candidate can run as it is.
    pub missing: Vec<String>,
    /// Programs this machine has under another name, put right when the command is used:
    /// `("fd", "fdfind")`. They do not count as missing.
    pub renames: Vec<(String, String)>,
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

/// Lowercase and drop the accents of Spanish letters (`ñ` included), to compare against
/// `STOP_WORDS` and the synonym table.
fn fold(word: &str) -> String {
    word.chars()
        .map(|c| match c {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' | 'ü' => 'u',
            'ñ' => 'n',
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

/// Question words that tldr says differently, from `data/synonyms.toml`, accents removed.
fn synonyms() -> &'static HashMap<String, Vec<String>> {
    static TABLE: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();
    TABLE.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct File {
            synonyms: HashMap<String, Vec<String>>,
        }
        toml::from_str::<File>(include_str!("../data/synonyms.toml"))
            .expect("data/synonyms.toml is valid")
            .synonyms
    })
}

/// The question terms plus what the synonym table adds for them (each word once, in order).
pub fn expand(terms: Vec<String>) -> Vec<String> {
    let table = synonyms();
    let mut out = terms.clone();
    for t in &terms {
        for extra in table.get(&fold(t)).into_iter().flatten() {
            if !out.contains(extra) {
                out.push(extra.clone());
            }
        }
    }
    out
}

/// tldr also lists keys typed inside an interactive program (`fdisk`'s `<n>`): they say what
/// they do, and by meaning they are the best match for "create a partition", but they are not
/// something to type at a prompt.
fn runnable(e: &Entry) -> bool {
    !e.cmd.trim_start().starts_with('<')
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
    let qvec = embed::question_vector(question);
    if qvec.is_some() {
        embed_new_entries(db);
    }
    ask_with(db, question, limit, &PARAMS, qvec.as_deref())
}

/// Your cheats are not part of `update`: give the ones you just wrote a vector too.
fn embed_new_entries(db: &Db) {
    let Ok(missing) = db.missing_vectors() else { return };
    if missing.is_empty() || missing.len() > EMBED_ON_THE_FLY {
        return;
    }
    let texts: Vec<String> = missing.iter().map(|(_, p)| format!("passage: {p}")).collect();
    let Some(Ok(vectors)) = embed::with_engine(|e| e.embed(&texts)) else {
        return;
    };
    let rows: Vec<(i64, Vec<i8>)> = missing
        .iter()
        .zip(&vectors)
        .map(|((key, _), v)| (*key, semantic::quantize(v)))
        .collect();
    let _ = db.put_vectors(&rows);
}

/// Entries closest in meaning to the question, as candidates (deduplicated by shape).
fn by_meaning(db: &Db, qvec: &[f32]) -> Result<Vec<Candidate>> {
    let index = db.vector_index()?;
    if index.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = index
        .top_k(qvec, VECTOR_POOL)
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    let mut seen = HashSet::new();
    Ok(db
        .entries_by_ids(&ids)?
        .into_iter()
        .filter(|e| runnable(e) && seen.insert(placeholders::shape(&e.cmd)))
        .take(POOL)
        .map(|entry| Candidate {
            entry,
            score: 0.0,
            missing: Vec::new(),
            renames: Vec::new(),
        })
        .collect())
}

fn identity(c: &Candidate) -> (&'static str, String) {
    (c.entry.source.db_name(), c.entry.cmd.clone())
}

/// `qvec` is the question's embedding; without it (or without stored vectors) this is BM25 only.
pub fn ask_with(
    db: &Db,
    question: &str,
    limit: usize,
    params: &Params,
    qvec: Option<&[f32]>,
) -> Result<Vec<Candidate>> {
    let words = if params.synonyms {
        expand(terms(question))
    } else {
        terms(question)
    };
    let mut ranked: Vec<Candidate> = db
        .fts_search_words(&words, POOL, params.weights)?
        .into_iter()
        .filter(|(entry, _)| runnable(entry))
        .map(|(entry, score)| Candidate {
            score: score * boost(&entry, params),
            entry,
            missing: Vec::new(),
            renames: Vec::new(),
        })
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));

    let mut seen = HashSet::new();
    ranked.retain(|c| seen.insert(placeholders::shape(&c.entry.cmd)));
    if let Some(qvec) = qvec {
        // a question of only stop words finds nothing by BM25 and must stay that way
        if !ranked.is_empty() || !words.is_empty() {
            ranked = fuse(ranked, by_meaning(db, qvec)?, &params.fusion);
        }
    }
    if params.per_tool > 0 {
        ranked = spread_tools(ranked, params.per_tool);
    }
    ranked.truncate(limit);
    // looked up after trimming: only the candidates that will be shown cost a PATH search
    for c in &mut ranked {
        let adapted = packages::adapt(&c.entry.cmd);
        c.missing = system::missing_binaries(&adapted.command);
        c.renames = adapted.renames;
    }
    Ok(ranked)
}

/// Keeps the order, but lets at most `max` tldr entries of one tool through at first: the rest
/// follow after the others. `tar` has a dozen examples and would otherwise fill the whole list.
/// Your cheats and history are never held back.
fn spread_tools(ranked: Vec<Candidate>, max: usize) -> Vec<Candidate> {
    let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let (mut first, mut later) = (Vec::new(), Vec::new());
    for c in ranked {
        if c.entry.source != Source::Tldr {
            first.push(c);
            continue;
        }
        let n = count.entry(c.entry.tool.clone()).or_insert(0);
        *n += 1;
        if *n <= max {
            first.push(c);
        } else {
            later.push(c);
        }
    }
    first.extend(later);
    first
}

/// BM25 (already boosted: your cheats and frequent history lead it) and meaning, fused by rank.
fn fuse(bm25: Vec<Candidate>, meaning: Vec<Candidate>, fusion: &Fusion) -> Vec<Candidate> {
    let rankings = [
        (bm25.iter().map(identity).collect::<Vec<_>>(), fusion.bm25),
        (meaning.iter().map(identity).collect::<Vec<_>>(), fusion.meaning),
    ];
    let mut by_key: std::collections::HashMap<_, Candidate> = std::collections::HashMap::new();
    for c in bm25.into_iter().chain(meaning) {
        by_key.entry(identity(&c)).or_insert(c);
    }
    let mut seen = HashSet::new();
    semantic::rrf(&rankings, fusion.k)
        .into_iter()
        .filter_map(|(key, score)| {
            let mut c = by_key.remove(&key)?;
            c.score = score;
            // the same example in two languages is one candidate
            seen.insert(placeholders::shape(&c.entry.cmd)).then_some(c)
        })
        .collect()
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
    fn keys_typed_inside_a_program_are_not_suggested() {
        let db = db_with(
            vec![
                entry("fdisk", "<n>", "create a new partition", "", Source::Tldr, 0),
                entry(
                    "fdisk",
                    "sudo fdisk {{/dev/sdX}}",
                    "partition manipulator",
                    "",
                    Source::Tldr,
                    0,
                ),
            ],
            vec![],
            vec![],
        );
        let top = ask(&db, "create a new partition", 5).unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].entry.cmd, "sudo fdisk {{/dev/sdX}}");
    }

    fn tool_of(tool: &str, cmd: &str, source: Source) -> Candidate {
        let mut c = cand(cmd, source);
        c.entry.tool = tool.into();
        c
    }

    #[test]
    fn one_tool_does_not_fill_the_list_but_your_own_commands_are_never_held_back() {
        let ranked = vec![
            tool_of("tar", "tar a", Source::Tldr),
            tool_of("tar", "tar b", Source::Tldr),
            tool_of("ssh", "ssh a", Source::Cheat),
            tool_of("ssh", "ssh b", Source::Cheat),
            tool_of("zip", "zip a", Source::Tldr),
        ];
        let cmds: Vec<String> = spread_tools(ranked, 1).into_iter().map(|c| c.entry.cmd).collect();
        assert_eq!(cmds, vec!["tar a", "ssh a", "ssh b", "zip a", "tar b"]);
    }

    fn cand(cmd: &str, source: Source) -> Candidate {
        Candidate {
            entry: entry("t", cmd, "", "", source, 0),
            score: 0.0,
            missing: Vec::new(),
            renames: Vec::new(),
        }
    }

    #[test]
    fn fusion_lifts_what_both_rankings_list_and_keeps_one_per_shape() {
        let bm25 = vec![
            cand("a {{x}}", Source::Tldr),
            cand("b", Source::Tldr),
            cand("c", Source::Cheat),
        ];
        let meaning = vec![
            cand("b", Source::Tldr),
            cand("a {{y}}", Source::Tldr),
            cand("d", Source::Tldr),
        ];
        let fused = fuse(bm25, meaning, &PARAMS.fusion);
        let cmds: Vec<&str> = fused.iter().map(|c| c.entry.cmd.as_str()).collect();
        // `a {{x}}` and `a {{y}}` have the same shape: the better ranked one stands for both
        assert_eq!(cmds, vec!["b", "a {{y}}", "d", "c"]);
        assert!(fused.windows(2).all(|w| w[0].score >= w[1].score));
    }

    #[test]
    fn synonyms_add_words_and_keep_the_original_ones_first() {
        let out = expand(terms("crear una carpeta"));
        assert_eq!(&out[..2], ["crear", "carpeta"]);
        assert!(out.contains(&"directory".to_string()) && out.contains(&"create".to_string()));
        // accents do not matter to the lookup, and an unknown word adds nothing
        assert!(expand(terms("contraseña")).contains(&"password".to_string()));
        assert_eq!(expand(terms("zzz")), vec!["zzz"]);
        // no word twice even when two question words share a synonym
        let out = expand(terms("borrar eliminar"));
        assert_eq!(out.iter().filter(|w| *w == "remove").count(), 1);
    }

    #[test]
    fn synonyms_let_a_spanish_question_reach_an_english_description() {
        let db = db_with(
            vec![entry(
                "mkdir",
                "mkdir {{dir}}",
                "create directories",
                "",
                Source::Tldr,
                0,
            )],
            vec![],
            vec![],
        );
        assert_eq!(ask(&db, "crear una carpeta", 5).unwrap().len(), 1);
        let mut off = PARAMS;
        off.synonyms = false;
        assert!(ask_with(&db, "crear una carpeta", 5, &off, None)
            .unwrap()
            .is_empty());
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

    /// Candidates say whether their programs are installed, without changing the order.
    #[test]
    fn candidates_are_marked_available_or_missing() {
        let db = db_with(
            vec![
                entry(
                    "pistasho-no-such-tool",
                    "pistasho-no-such-tool --size {{dir}}",
                    "disk usage browser",
                    "",
                    Source::Tldr,
                    0,
                ),
                entry("ls", "ls -l | sort", "disk usage listing", "", Source::Tldr, 0),
            ],
            vec![],
            vec![],
        );
        let found = ask(&db, "disk usage", 5).unwrap();
        let by_tool = |t: &str| found.iter().find(|c| c.entry.tool == t).unwrap();
        assert_eq!(
            by_tool("pistasho-no-such-tool").missing,
            vec!["pistasho-no-such-tool"]
        );
        assert!(by_tool("ls").missing.is_empty(), "ls and sort are installed");
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
