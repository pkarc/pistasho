//! `pistasho eval`: measures how often the searcher puts the correct tool on top.

use crate::i18n::tr;
use crate::index::Entry;
use crate::search::Searcher;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct EvalFile {
    q: Vec<Question>,
}

#[derive(Deserialize)]
struct Question {
    question: String,
    /// Tools that count as a correct answer.
    tools: Vec<String>,
}

pub fn run(path: &Path, entries: &[Entry]) -> Result<()> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("{} {}", tr("could not read", "no pude leer"), path.display()))?;
    let file: EvalFile = toml::from_str(&text)?;
    let mut searcher = Searcher::new(entries);

    let (mut top1, mut top3, mut misses) = (0, 0, Vec::new());
    for q in &file.q {
        let ranked = searcher.rank(entries, &q.question, 3);
        let tools: Vec<&str> = ranked.iter().map(|&i| entries[i].tool.as_str()).collect();
        let hit = |t: &&str| q.tools.iter().any(|h| h == t);
        if tools.first().is_some_and(hit) {
            top1 += 1;
        }
        if tools.iter().any(hit) {
            top3 += 1;
        } else {
            misses.push((q.question.as_str(), tools.join(", ")));
        }
    }

    let n = file.q.len().max(1) as f64;
    println!("{} {}", tr("questions:", "preguntas:"), file.q.len());
    println!("top-1:     {:.0}%", top1 as f64 / n * 100.0);
    println!("top-3:     {:.0}%", top3 as f64 / n * 100.0);
    if !misses.is_empty() {
        println!(
            "\n{}",
            tr("misses (what came out on top):", "fallos (lo que salió arriba):")
        );
        for (p, got) in misses {
            println!(
                "  · {p}  →  {}",
                if got.is_empty() {
                    tr("nothing", "nada")
                } else {
                    &got
                }
            );
        }
    }
    Ok(())
}
