//! SQLite store at `~/.local/share/pistasho/pistasho.db`.
//!
//! Holds the command entries (tldr, cheats, history) and a full-text index over them
//! (FTS5, BM25, accent-insensitive). It is derived data: `pistasho update` rebuilds the tldr
//! rows, and cheats and history are re-synced whenever their files change.

use crate::index::{Entry, Source};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

/// Bump when the tables change; a different version resets the database.
pub const SCHEMA_VERSION: i64 = 1;

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
    uses        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS entries_source ON entries(source);
CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
    description, alt, tool,
    content = 'entries', content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2'
);
CREATE TRIGGER IF NOT EXISTS entries_ai AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts(rowid, description, alt, tool)
    VALUES (new.id, new.description, new.alt, new.tool);
END;
CREATE TRIGGER IF NOT EXISTS entries_ad AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts(entries_fts, rowid, description, alt, tool)
    VALUES ('delete', old.id, old.description, old.alt, old.tool);
END;
";

const DROP_ALL: &str = "
DROP TRIGGER IF EXISTS entries_ai;
DROP TRIGGER IF EXISTS entries_ad;
DROP TABLE IF EXISTS entries_fts;
DROP TABLE IF EXISTS entries;
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
                "INSERT INTO entries(cmd, description, alt, tool, source, origin, uses)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for e in entries {
                insert.execute(params![
                    e.cmd,
                    e.desc,
                    e.alt,
                    e.tool,
                    e.source.db_name(),
                    e.origin,
                    e.count
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

    /// Full-text search ranked by BM25. Returns `(entry, score)`, best first; higher is better.
    /// Not used by a command yet: `pistasho ask` (task 2.2) is its caller.
    #[allow(dead_code)]
    pub fn fts_search(&self, text: &str, limit: usize) -> Result<Vec<(Entry, f64)>> {
        let Some(query) = fts_query(text) else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS}, bm25(entries_fts) FROM entries_fts
             JOIN entries e ON e.id = entries_fts.rowid
             WHERE entries_fts MATCH ?1
             ORDER BY bm25(entries_fts) LIMIT ?2"
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

/// Light stemming for prefix search: words of up to 4 characters are kept whole, longer ones
/// lose their last 3 (never going below 4). That is enough for `comprimir` → `compri` to reach
/// `comprime` / `comprimido`, and `compresión` → `compres` to reach `compression` and
/// `compress`, in Spanish and English alike. The cut is a heuristic to tune with `pistasho eval`.
#[allow(dead_code)] // used through `fts_query`, which `pistasho ask` (task 2.2) will call
fn stem_prefix(word: &str) -> String {
    let n = word.chars().count();
    let keep = if n <= 4 { n } else { (n - 3).max(4) };
    word.chars().take(keep).collect()
}

/// Turns free text into an FTS5 query: every word becomes a quoted, stemmed prefix term
/// (`"compri"*`), joined with OR so BM25 ranks the entries that match the most words first.
/// Punctuation is dropped, which also keeps user input from being read as FTS5 syntax.
#[allow(dead_code)] // see `stem_prefix`
pub fn fts_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| format!("\"{}\"*", stem_prefix(&w.to_lowercase())))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" OR "))
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
            Some("\"compri\"* OR \"carp\"*".into())
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
        assert_eq!(stem_prefix("sockets"), "sock");
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
}
