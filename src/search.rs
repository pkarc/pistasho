//! Fuzzy search with nucleo-matcher, plus ranking.

use crate::index::{Entry, Source};
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config as MatcherConfig, Matcher, Utf32Str};

pub struct Searcher {
    matcher: Matcher,
    buf: Vec<char>,
    /// Precomputed search text per entry.
    hay: Vec<String>,
}

impl Searcher {
    pub fn new(entries: &[Entry]) -> Self {
        let hay = entries
            .iter()
            .map(|e| format!("{} {} {} {}", e.tool, e.cmd, e.desc, e.alt))
            .collect();
        Self {
            matcher: Matcher::new(MatcherConfig::DEFAULT),
            buf: Vec::new(),
            hay,
        }
    }

    /// Returns indices into `entries` ordered best to worst, at most `limit`.
    pub fn rank(&mut self, entries: &[Entry], query: &str, limit: usize) -> Vec<usize> {
        let query = query.trim();
        if query.is_empty() {
            return empty_query_order(entries, limit);
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let first = query.split_whitespace().next().unwrap_or("").to_lowercase();

        let mut scored: Vec<(usize, i64)> = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let Some(score) = pattern.score(Utf32Str::new(&self.hay[i], &mut self.buf), &mut self.matcher)
            else {
                continue;
            };
            scored.push((i, score as i64 + bonus(e, &first)));
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        scored.truncate(limit);
        scored.into_iter().map(|(i, _)| i).collect()
    }
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
