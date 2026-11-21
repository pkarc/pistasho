//! SQLite store at `~/.local/share/pistasho/pistasho.db`.
//!
//! Holds the command entries (tldr, cheats, history) and a full-text index over them
//! (FTS5, BM25, accent-insensitive). It is derived data: `pistasho update` rebuilds the tldr
//! rows, and cheats and history are re-synced whenever their files change.

use crate::index::{Entry, Source};
use crate::placeholders;
use crate::semantic::{self, VectorIndex};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

/// Bump when the tables change; a different version resets the database.
pub const SCHEMA_VERSION: i64 = 3;

/// `description` and `uses` instead of `desc` and `count` to stay clear of SQL keywords.
/// The FTS table is "external content": it indexes `entries` without copying it, and the
/// triggers keep the two in step.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entries (
    id          INTEGER PRIMARY KEY,
    cmd         TEXT NOT NULL,
    description TEXT NOT NULL,
    alt         TEXT NOT NULL DEFAULT '',
    tool        TEXT NOT NULL,
    source      TEXT NOT NULL,
    origin      TEXT NOT NULL,
    uses        INTEGER NOT NULL DEFAULT 0,
    cmd_words   TEXT NOT NULL DEFAULT '',
    vkey        INTEGER
);
CREATE INDEX IF NOT EXISTS entries_source ON entries(source);
CREATE INDEX IF NOT EXISTS entries_vkey ON entries(vkey);
-- embeddings, keyed by a hash of the entry's passage (see semantic.rs); i8, DIM bytes each
CREATE TABLE IF NOT EXISTS vectors (
    key INTEGER PRIMARY KEY,
    v   BLOB NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    description, alt, tool, cmd_words,
    content = 'entries', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, description, alt, tool, cmd_words)
    VALUES (new.id, new.description, new.alt, new.tool, new.cmd_words);
END;
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, description, alt, tool, cmd_words)
    VALUES ('delete', old.id, old.description, old.alt, old.tool, old.cmd_words);
END;
";

const DROP_ALL: &str = "
DROP TRIGGER IF EXISTS entries_ai;
DROP TRIGGER IF EXISTS entries_ad;
DROP TABLE IF EXISTS entries_fts;
DROP TABLE IF EXISTS entries;
DROP TABLE IF EXISTS vectors;
DROP TABLE IF EXISTS meta;
";

const COLUMNS: &str = "e.cmd, e.description, e.alt, e.tool, e.source, e.origin, e.uses";

pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path).with_context(|| {
            format!(
                "{} {}",
                crate::i18n::tr("could not open", "no pude abrir"),
                path.display()
            )
        })?;
        // several shells may open it at once; readers must not block the writer in WAL mode
        conn.busy_timeout(Duration::from_secs(3))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        let db = Self { conn };
        let version = db
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            // no `meta` table yet: a fresh database
            .unwrap_or(None)
            .and_then(|v| v.parse::<i64>().ok());
        if version.is_some_and(|v| v != SCHEMA_VERSION) {
            db.conn.execute_batch(DROP_ALL)?;
        }
        db.conn.execute_batch(SCHEMA)?;
        db.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
        Ok(db)
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    /// Replaces every row of one source (and the meta keys that describe it) in one transaction.
    pub fn replace_source(&mut self, source: Source, entries: &[Entry], meta: &[(&str, &str)]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM entries WHERE source = ?1", [source.db_name()])?;
        {
            let mut insert = tx.prepare(
                "INSERT INTO entries(cmd, description, alt, tool, source, origin, uses, cmd_words, vkey)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )?;
            for e in entries {
                // history descriptions are generated labels ("used 3 times", in the UI
                // language), not text worth searching, and are rebuilt on load anyway
                let description = if e.source == Source::Hist {
                    ""
                } else {
                    e.desc.as_str()
                };
                insert.execute(params![
                    e.cmd,
                    description,
                    e.alt,
                    e.tool,
                    e.source.db_name(),
                    e.origin,
                    e.count,
                    placeholders::plain_words(&e.cmd),
                    semantic::embeds(e).then(|| semantic::key(&semantic::passage_text(
                        &e.tool,
                        description,
                        &e.alt
                    )))
                ])?;
            }
        }
        for (k, v) in meta {
            tx.execute(
                "INSERT INTO meta(key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [k, v],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn count_source(&self, source: Source) -> Result<usize> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM entries WHERE source = ?1",
            [source.db_name()],
            |r| r.get::<_, i64>(0),
        )? as usize)
    }

    /// Every entry, in the order the fuzzy searcher expects: cheats, history, then tldr.
    pub fn load_all(&self) -> Result<Vec<Entry>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM entries e
             ORDER BY CASE e.source WHEN 'cheat' THEN 0 WHEN 'hist' THEN 1 ELSE 2 END, e.id"
        ))?;
        let rows = stmt.query_map([], row_to_entry)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Passages (and their keys) of the entries that have no vector yet, longest last so batches
    /// of similar length waste little padding.
    pub fn missing_vectors(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT e.vkey, e.tool, e.description, e.alt FROM entries e
             LEFT JOIN vectors v ON v.key = e.vkey
             WHERE e.vkey IS NOT NULL AND v.key IS NULL",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                semantic::passage_text(
                    &r.get::<_, String>(1)?,
                    &r.get::<_, String>(2)?,
                    &r.get::<_, String>(3)?,
                ),
            ))
        })?;
        let mut out = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        out.sort_by_key(|(_, p)| p.len());
        Ok(out)
    }

    pub fn put_vectors(&self, vectors: &[(i64, Vec<i8>)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut insert = tx.prepare("INSERT OR REPLACE INTO vectors(key, v) VALUES (?1, ?2)")?;
            for (key, v) in vectors {
                let bytes: Vec<u8> = v.iter().map(|&b| b as u8).collect();
                insert.execute(params![key, bytes])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Forgets the vectors no entry points to any more (the text changed or the page is gone).
    pub fn prune_vectors(&self) -> Result<()> {
        self.conn.execute(
            "DELETE FROM vectors WHERE key NOT IN (SELECT vkey FROM entries WHERE vkey IS NOT NULL)",
            [],
        )?;
        Ok(())
    }

    pub fn clear_vectors(&self) -> Result<()> {
        self.conn.execute("DELETE FROM vectors", [])?;
        Ok(())
    }

    /// `(entries that should have a vector, entries that have one)`.
    pub fn vector_coverage(&self) -> Result<(usize, usize)> {
        let (all, have): (i64, i64) = self.conn.query_row(
            "SELECT COUNT(e.vkey), COUNT(v.key) FROM entries e LEFT JOIN vectors v ON v.key = e.vkey",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((all as usize, have as usize))
    }

    /// Every vector, with the id of its entry (an entry that shares a passage shares a vector).
    pub fn vector_index(&self) -> Result<VectorIndex> {
        let mut stmt = self
            .conn
            .prepare("SELECT e.id, v.v FROM entries e JOIN vectors v ON v.key = e.vkey")?;
        let mut rows = stmt.query([])?;
        let mut index = VectorIndex::default();
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let blob = r.get_ref(1)?.as_blob()?;
            index.push(id, blob);
        }
        Ok(index)
    }

    /// The entries with these ids, in the same order (ids that no longer exist are skipped).
    pub fn entries_by_ids(&self, ids: &[i64]) -> Result<Vec<Entry>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {COLUMNS} FROM entries e WHERE e.id = ?1"))?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(e) = stmt.query_row([id], row_to_entry).optional()? {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Full-text search of free text with the default weights; see [`Db::fts_search_words`].
    #[cfg(test)]
    pub fn fts_search(&self, text: &str, limit: usize) -> Result<Vec<(Entry, f64)>> {
        self.fts_search_words(&words(text), limit, DEFAULT_WEIGHTS)
    }

    /// Full-text search ranked by BM25. Returns `(entry, score)`, best first; higher is better.
    /// `weights` scale each indexed column (`description`, `alt`, `tool`, `cmd_words`).
    pub fn fts_search_words(
        &self,
        words: &[String],
        limit: usize,
        weights: Weights,
    ) -> Result<Vec<(Entry, f64)>> {
        let Some(query) = fts_query_from_words(words) else {
            return Ok(Vec::new());
        };
        let [w_desc, w_alt, w_tool, w_cmd] = weights;
        let bm25 = format!("bm25(entries_fts, {w_desc}, {w_alt}, {w_tool}, {w_cmd})");
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS}, {bm25} FROM entries_fts
             JOIN entries e ON e.id = entries_fts.rowid
             WHERE entries_fts MATCH ?1
             ORDER BY {bm25} LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![query, limit as i64], |r| {
            // bm25() is lower-is-better (and negative); flip it
            Ok((row_to_entry(r)?, -r.get::<_, f64>(7)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn row_to_entry(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        cmd: r.get(0)?,
        desc: r.get(1)?,
        alt: r.get(2)?,
        tool: r.get(3)?,
        source: Source::from_db_name(&r.get::<_, String>(4)?).unwrap_or(Source::Tldr),
        origin: r.get(5)?,
        count: r.get(6)?,
    })
}

/// Light stemming for prefix search: words of up to 6 characters are kept whole, longer ones
/// lose their last 3 (never going below 6). That is enough for `comprimir` → `compri` to reach
/// `comprime` / `comprimido`, and `compresión` → `compres` to reach `compression` and
/// `compress`, in Spanish and English alike. Chosen with `pistasho eval --mode bm25` over a
/// grid of cuts and minimums (docs/eval.md); shorter minimums were worse because short
/// prefixes match unrelated tool names.
fn stem_prefix(word: &str) -> String {
    const CUT: usize = 3;
    const MIN_KEEP: usize = 6;
    let n = word.chars().count();
    let keep = if n <= MIN_KEEP { n } else { (n - CUT).max(MIN_KEEP) };
    word.chars().take(keep).collect()
}

/// Weights of the BM25 columns: `description`, `alt`, `tool`, `cmd_words`.
pub type Weights = [f64; 4];

/// From a grid over 101 questions (docs/eval.md): description 3, alt 2, tool 4, command words 2.
/// The surface is flat (BM25 is not very sensitive to these), so do not chase decimals.
pub const DEFAULT_WEIGHTS: Weights = [3.0, 2.0, 4.0, 2.0];

/// Lowercase alphanumeric words of free text, in order.
pub fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Turns words into an FTS5 query: each one becomes a quoted, stemmed prefix term (`"compri"*`),
/// joined with OR so BM25 ranks the entries that match the most words first. The words come
/// from [`words`], so punctuation (and with it any FTS5 syntax in the user's text) is gone.
///
/// Prefix matching applies to every column, tool names included. Restricting the tool column to
/// exact matches looked better on 40 questions and worse on 100 (top-3 50% → 39%): partial tool
/// names are how `dock` or `compose` reach `docker-compose`.
pub fn fts_query_from_words(words: &[String]) -> Option<String> {
    let terms: Vec<String> = words.iter().map(|w| format!("\"{}\"*", stem_prefix(w))).collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

#[cfg(test)]
pub fn fts_query(text: &str) -> Option<String> {
    fts_query_from_words(&words(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool: &str, cmd: &str, desc: &str, alt: &str, source: Source) -> Entry {
        Entry {
            cmd: cmd.into(),
            desc: desc.into(),
            alt: alt.into(),
            tool: tool.into(),
            source,
            origin: "test".into(),
            count: 0,
        }
    }

    fn tldr_sample() -> Vec<Entry> {
        vec![
            entry(
                "zip",
                "zip -r {{file}}.zip {{dir}}",
                "Compress a folder into a zip",
                "Comprime una carpeta en un zip",
                Source::Tldr,
            ),
            entry(
                "gzip",
                "gzip {{file}}",
                "Compression of a single file",
                "Compresión de un solo archivo",
                Source::Tldr,
            ),
            entry(
                "tar",
                "tar xvf {{file}}",
                "Extract an archive",
                "Extrae un archivo comprimido",
                Source::Tldr,
            ),
            entry(
                "ss",
                "ss -tlnp",
                "Show listening sockets",
                "Muestra los sockets en escucha",
                Source::Tldr,
            ),
        ]
    }

    fn tools(hits: &[(Entry, f64)]) -> Vec<&str> {
        hits.iter().map(|(e, _)| e.tool.as_str()).collect()
    }

    #[test]
    fn builds_fts_queries() {
        assert_eq!(
            fts_query("Comprimir carpeta"),
            Some("\"compri\"* OR \"carpet\"*".into())
        );
        assert_eq!(fts_query("compresión"), Some("\"compres\"*".into()));
        assert_eq!(
            fts_query("  ¿cómo veo mi ip?  "),
            Some("\"cómo\"* OR \"veo\"* OR \"mi\"* OR \"ip\"*".into())
        );
        // quotes and FTS operators in the input are just punctuation
        assert_eq!(
            fts_query("a\" OR \"b*"),
            Some("\"a\"* OR \"or\"* OR \"b\"*".into())
        );
        assert_eq!(fts_query("  !?  "), None);
    }

    #[test]
    fn stores_and_loads_in_searcher_order() {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[("languages", "en,es")])
            .unwrap();
        db.replace_source(
            Source::Hist,
            &[entry("git", "git status", "x", "", Source::Hist)],
            &[],
        )
        .unwrap();
        db.replace_source(
            Source::Cheat,
            &[entry(
                "fuser",
                "fuser -k {{port}}/tcp",
                "kill port",
                "net",
                Source::Cheat,
            )],
            &[],
        )
        .unwrap();
        let all = db.load_all().unwrap();
        let sources: Vec<Source> = all.iter().map(|e| e.source).collect();
        assert_eq!(&sources[..2], &[Source::Cheat, Source::Hist]);
        assert_eq!(all.len(), 6);
        assert_eq!(db.count_source(Source::Tldr).unwrap(), 4);
        assert_eq!(db.meta("languages").unwrap().as_deref(), Some("en,es"));
        assert_eq!(db.meta("missing").unwrap(), None);
    }

    #[test]
    fn replace_is_idempotent_and_leaves_other_sources() {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
        db.replace_source(
            Source::Cheat,
            &[entry("fuser", "fuser -k", "kill port", "", Source::Cheat)],
            &[],
        )
        .unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
        assert_eq!(db.count_source(Source::Tldr).unwrap(), 4);
        assert_eq!(db.count_source(Source::Cheat).unwrap(), 1);
    }

    /// `alt` carries the other-language descriptions, so a Spanish word finds an entry whose
    /// main description is English.
    #[test]
    fn spanish_query_finds_english_entry_through_alt() {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
        let hits = db.fts_search("carpeta", 10).unwrap();
        assert_eq!(tools(&hits), vec!["zip"]);
        assert!(
            hits[0].0.desc.starts_with("Compress"),
            "the displayed description stays English"
        );
    }

    #[test]
    fn accents_are_ignored_both_ways() {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
        // typed without the accent, stored with it ("Compresión")
        let plain = tools(&db.fts_search("compresion", 10).unwrap()).join(",");
        // typed with the accent: same results
        assert_eq!(tools(&db.fts_search("compresión", 10).unwrap()).join(","), plain);
        assert!(plain.contains("gzip"), "{plain}");
        // and the other way: a search for "carpeta" is unaffected by accents elsewhere
        assert_eq!(tools(&db.fts_search("CARPETA", 10).unwrap()), vec!["zip"]);
    }

    /// The 2.1 acceptance test: "compresión" finds "compression" (English description) and
    /// "comprimir" finds "comprime" / "comprimido" (Spanish `alt`), and neither drags in
    /// unrelated entries.
    #[test]
    fn compression_words_find_each_other_by_stem() {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
        let hits = db.fts_search("compresión", 10).unwrap();
        let found = tools(&hits);
        assert!(found.contains(&"gzip") && found.contains(&"zip"), "{found:?}");
        assert!(!found.contains(&"ss"), "{found:?}");
        let hits = db.fts_search("comprimir", 10).unwrap();
        let found = tools(&hits);
        assert!(found.contains(&"zip") && found.contains(&"tar"), "{found:?}");
        assert!(!found.contains(&"ss"), "{found:?}");
        let hits = db.fts_search("compress", 10).unwrap();
        let found = tools(&hits);
        assert!(found.contains(&"zip") && found.contains(&"gzip"), "{found:?}");
    }

    #[test]
    fn stems_words() {
        assert_eq!(stem_prefix("ip"), "ip");
        assert_eq!(stem_prefix("kill"), "kill");
        assert_eq!(stem_prefix("puerto"), "puerto");
        assert_eq!(stem_prefix("sockets"), "socket");
        assert_eq!(stem_prefix("directorio"), "directo");
        assert_eq!(stem_prefix("comprimir"), "compri");
        assert_eq!(stem_prefix("compresión"), "compres");
    }

    #[test]
    fn more_matching_words_rank_higher() {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
        let hits = db.fts_search("comprime carpeta zip", 10).unwrap();
        assert_eq!(hits[0].0.tool, "zip");
        assert!(hits[0].1 > 0.0, "scores are flipped to higher-is-better");
    }

    #[test]
    fn replaced_rows_leave_no_stale_fts_hits() {
        let mut db = Db::open_in_memory().unwrap();
        db.replace_source(
            Source::Cheat,
            &[entry(
                "fuser",
                "fuser -k",
                "kill the process on a port",
                "",
                Source::Cheat,
            )],
            &[],
        )
        .unwrap();
        assert_eq!(db.fts_search("kill", 10).unwrap().len(), 1);
        db.replace_source(
            Source::Cheat,
            &[entry("ss", "ss -tl", "list sockets", "", Source::Cheat)],
            &[],
        )
        .unwrap();
        assert!(db.fts_search("kill", 10).unwrap().is_empty());
        assert_eq!(tools(&db.fts_search("sockets", 10).unwrap()), vec!["ss"]);
    }

    #[test]
    fn empty_or_symbol_only_query_returns_nothing() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.fts_search("", 10).unwrap().is_empty());
        assert!(db.fts_search("?!", 10).unwrap().is_empty());
    }

    #[test]
    fn other_schema_version_resets_the_database() {
        let dir = std::env::temp_dir().join(format!("pistasho-test-{}", std::process::id()));
        let path = dir.join("pistasho.db");
        {
            let mut db = Db::open(&path).unwrap();
            db.replace_source(Source::Tldr, &tldr_sample(), &[]).unwrap();
            db.set_meta("schema_version", "0").unwrap();
        }
        let db = Db::open(&path).unwrap();
        assert_eq!(db.count_source(Source::Tldr).unwrap(), 0, "old data is dropped");
        assert_eq!(
            db.meta("schema_version").unwrap(),
            Some(SCHEMA_VERSION.to_string())
        );
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn vectors_follow_the_text_of_the_entry_not_its_row() {
        let mut db = Db::open_in_memory().unwrap();
        let tldr = tldr_sample();
        db.replace_source(Source::Tldr, &tldr, &[]).unwrap();
        db.replace_source(Source::Hist, &[entry("ls", "ls -la", "", "", Source::Hist)], &[])
            .unwrap();
        // history is not embedded; every tldr entry waits for a vector
        let missing = db.missing_vectors().unwrap();
        assert_eq!(missing.len(), tldr.len());
        assert_eq!(db.vector_coverage().unwrap(), (tldr.len(), 0));

        let rows: Vec<(i64, Vec<i8>)> = missing
            .iter()
            .map(|(k, _)| (*k, vec![1i8; semantic::DIM]))
            .collect();
        db.put_vectors(&rows).unwrap();
        assert!(db.missing_vectors().unwrap().is_empty());
        assert_eq!(db.vector_index().unwrap().ids.len(), tldr.len());

        // `update` replaces the rows; unchanged text keeps its vector and needs no new embedding
        db.replace_source(Source::Tldr, &tldr, &[]).unwrap();
        assert!(db.missing_vectors().unwrap().is_empty());

        // an edited description is a new passage; the old vector is pruned
        let mut edited = tldr.clone();
        edited[0].desc = "Something else entirely".into();
        db.replace_source(Source::Tldr, &edited, &[]).unwrap();
        assert_eq!(db.missing_vectors().unwrap().len(), 1);
        db.prune_vectors().unwrap();
        assert_eq!(db.vector_coverage().unwrap().1, tldr.len() - 1);
    }

    #[test]
    fn entries_by_ids_keeps_the_order_asked_for() {
        let mut db = Db::open_in_memory().unwrap();
        let sample = tldr_sample();
        db.replace_source(Source::Tldr, &sample, &[]).unwrap();
        // rows of a fresh table are numbered from 1 in insertion order
        let got = db.entries_by_ids(&[3, 1, 999]).unwrap();
        assert_eq!(got.len(), 2, "a missing id is skipped");
        assert_eq!(got[0].cmd, sample[2].cmd);
        assert_eq!(got[1].cmd, sample[0].cmd);
    }
}
