//! Fuzzy search with nucleo-matcher, plus ranking.
//!
//! Typing is the hot path: the interface calls `rank` on every keystroke over ~35k entries.
//! Two things keep it fast: extending the query only rescores the entries that matched the
//! shorter one (see `Searcher::stack`), and big scans are split across threads.

use crate::index::{Entry, Source};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as MatcherConfig, Matcher, Utf32Str};

/// Below this many candidates a single thread is faster than spawning.
const PARALLEL_MIN: usize = 4096;
const MAX_THREADS: usize = 8;

/// Only this many characters of `Entry::alt` (other-language descriptions, then the page
/// summary) are searched. The tail is mostly the page summary, repeated on every example of a
/// page, and it doubled the cost of each keystroke. Measured with `pistasho eval` (docs/eval.md):
/// 150 keeps the quality and brings a keystroke from ~36 ms to ~25 ms; 60 starts to lose accuracy.
const ALT_SEARCH_CHARS: usize = 150;

pub struct Searcher {
    /// Precomputed search text per entry.
    hay: Vec<String>,
    /// Queries typed so far, each a prefix of the next, with the entries that matched.
    /// Extending a query can only shrink its matches, so the longest stored prefix gives
    /// the candidates for the new one. Backspace pops back to a shorter prefix.
    stack: Vec<(String, Vec<u32>)>,
}

impl Searcher {
    pub fn new(entries: &[Entry]) -> Self {
        let hay = entries
            .iter()
            .map(|e| {
                let alt: String = e.alt.chars().take(ALT_SEARCH_CHARS).collect();
                format!("{} {} {} {}", e.tool, e.cmd, e.desc, alt)
            })
            .collect();
        Self {
            hay,
            stack: Vec::new(),
        }
    }

    /// Returns indices into `entries` ordered best to worst, at most `limit`.
    pub fn rank(&mut self, entries: &[Entry], query: &str, limit: usize) -> Vec<usize> {
        let query = query.trim();
        if query.is_empty() {
            self.stack.clear();
            return empty_query_order(entries, limit);
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let first = query.split_whitespace().next().unwrap_or("").to_lowercase();

        // Operators like `!x`, `^x`, `x$` and `'x` do not narrow monotonically (`!ta` excludes
        // more than `!tar`), so they always scan everything.
        let narrowable = !query.contains(['!', '\'', '^', '$', '\\']);
        let base = if narrowable {
            self.candidates_for(query)
        } else {
            self.stack.clear();
            None
        };

        let scored = score_all(&pattern, &self.hay, base.as_deref());
        if narrowable {
            self.stack
                .push((query.to_string(), scored.iter().map(|&(i, _)| i).collect()));
        }

        let mut ranked: Vec<(usize, i64)> = scored
            .into_iter()
            .map(|(i, s)| (i as usize, s as i64 + bonus(&entries[i as usize], &first)))
            .collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(limit);
        ranked.into_iter().map(|(i, _)| i).collect()
    }

    /// Entries that can match `query`: the matches of the longest stored prefix of it,
    /// or `None` (scan everything) when there is none.
    fn candidates_for(&mut self, query: &str) -> Option<Vec<u32>> {
        while let Some((q, _)) = self.stack.last() {
            if query.starts_with(q.as_str()) {
                break;
            }
            self.stack.pop();
        }
        // asking again for the same query: replace it instead of stacking a duplicate
        if self.stack.last().is_some_and(|(q, _)| q == query) {
            self.stack.pop();
        }
        self.stack.last().map(|(_, m)| m.clone())
    }
}

/// Scores every candidate (all entries when `base` is `None`), in parallel when there are many.
/// Returns `(entry index, fuzzy score)` for the ones that match, in index order.
fn score_all(pattern: &Pattern, hay: &[String], base: Option<&[u32]>) -> Vec<(u32, u32)> {
    let n = base.map_or(hay.len(), |b| b.len());
    let threads = if n < PARALLEL_MIN {
        1
    } else {
        std::thread::available_parallelism()
            .map_or(1, |p| p.get())
            .min(MAX_THREADS)
    };
    let chunk = n.div_ceil(threads).max(1);

    let scan = |lo: usize, hi: usize| {
        let mut matcher = Matcher::new(MatcherConfig::DEFAULT);
        let mut buf = Vec::new();
        let mut out = Vec::new();
        for k in lo..hi {
            let i = base.map_or(k as u32, |b| b[k]);
            if let Some(score) = pattern.score(Utf32Str::new(&hay[i as usize], &mut buf), &mut matcher) {
                out.push((i, score));
            }
        }
        out
    };

    if threads == 1 {
        return scan(0, n);
    }
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let (lo, hi) = (t * chunk, ((t + 1) * chunk).min(n));
                let scan = &scan;
                scope.spawn(move || scan(lo.min(hi), hi))
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("search thread panicked"))
            .collect()
    })
}

/// Adjustments on top of the fuzzy score: your own entries first, frequently used ones rise,
/// and if the first word is the exact tool name, it wins.
fn bonus(e: &Entry, first_word: &str) -> i64 {
    let mut b = 0;
    if e.tool.eq_ignore_ascii_case(first_word) {
        b += 80;
    }
    match e.source {
        Source::Cheat => b += 40,
        Source::Hist => b += (e.count as i64 * 4).min(40),
        Source::Tldr => {}
    }
    b
}

/// With no text: your cheats, then the most recent history, then tldr.
fn empty_query_order(entries: &[Entry], limit: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..entries.len()).collect();
    idx.sort_by_key(|&i| match entries[i].source {
        Source::Cheat => 0,
        Source::Hist => 1,
        Source::Tldr => 2,
    });
    idx.truncate(limit);
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool: &str, cmd: &str, desc: &str, source: Source) -> Entry {
        Entry {
            cmd: cmd.into(),
            desc: desc.into(),
            alt: String::new(),
            tool: tool.into(),
            source,
            origin: String::new(),
            count: 0,
        }
    }

    fn sample() -> Vec<Entry> {
        let mut v = vec![
            entry(
                "tar",
                "tar czf {{file}}.tar.gz {{dir}}",
                "create a gzipped archive",
                Source::Tldr,
            ),
            entry("tar", "tar xvf {{file}}", "extract an archive", Source::Tldr),
            entry(
                "zip",
                "zip -r {{file}}.zip {{dir}}",
                "compress a folder",
                Source::Tldr,
            ),
            entry("ss", "ss -tlnp", "show listening sockets", Source::Tldr),
            entry(
                "fuser",
                "fuser -k {{port}}/tcp",
                "kill the process on a port",
                Source::Cheat,
            ),
            entry("git", "git status", "show the working tree status", Source::Hist),
        ];
        // enough entries to cross PARALLEL_MIN and exercise the threaded path
        for i in 0..5000 {
            v.push(entry(
                &format!("tool{i}"),
                &format!("tool{i} --flag"),
                "filler entry",
                Source::Tldr,
            ));
        }
        v
    }

    /// Narrowing must never change the result: typing a query key by key (and deleting) has
    /// to rank exactly like scanning everything from scratch.
    #[test]
    fn incremental_equals_fresh() {
        let entries = sample();
        let mut typed = Searcher::new(&entries);
        let queries = [
            "t",
            "ta",
            "tar",
            "tar ",
            "tar c",
            "tar cz",
            "tar czf",
            "tar cz",
            "tar",
            "ta",
            "zi",
            "zip -r",
            "zip",
            "kill port",
            "kill por",
            "git s",
            "tool49",
            "tool4",
        ];
        for q in queries {
            let fresh = Searcher::new(&entries).rank(&entries, q, 50);
            assert_eq!(typed.rank(&entries, q, 50), fresh, "query {q:?}");
        }
    }

    #[test]
    fn operators_scan_everything() {
        let entries = sample();
        let mut typed = Searcher::new(&entries);
        // `!` inverts: the result for `!tar` is a superset of the one for `!tarx`
        for q in ["tar", "!tar", "!tarx", "^tar", "ss$"] {
            let fresh = Searcher::new(&entries).rank(&entries, q, 20);
            assert_eq!(typed.rank(&entries, q, 20), fresh, "query {q:?}");
        }
    }

    #[test]
    fn repeated_query_is_stable() {
        let entries = sample();
        let mut s = Searcher::new(&entries);
        let a = s.rank(&entries, "tar c", 10);
        assert_eq!(s.rank(&entries, "tar c", 10), a);
        assert_eq!(s.rank(&entries, "tar c", 10), a);
    }

    #[test]
    fn exact_tool_name_wins() {
        let entries = sample();
        let ranked = Searcher::new(&entries).rank(&entries, "tar", 3);
        assert_eq!(entries[ranked[0]].tool, "tar");
    }
}
