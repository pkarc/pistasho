//! `pistasho eval`: measures how often the searcher puts the correct tool on top.
//!
//! Two rankers can be compared on the same questions: the fuzzy searcher of phase 1 and the
//! BM25 search of `pistasho ask`.

use crate::ask::{self, Params};
use crate::db::{Db, Weights};
use crate::i18n::tr;
use crate::index::Entry;
use crate::search::Searcher;
use anyhow::{Context, Result};
use clap::ValueEnum;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Fuzzy matching of phase 1 (`pistasho search`)
    Fuzzy,
    /// BM25 over the full-text index (`pistasho ask`)
    Bm25,
}

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

pub struct Report {
    pub questions: usize,
    pub top1: usize,
    pub top3: usize,
    /// `(question, tools that came out on top)` for the questions with no hit in the top 3.
    pub misses: Vec<(String, String)>,
}

impl Report {
    pub fn top1_pct(&self) -> f64 {
        self.top1 as f64 / self.questions.max(1) as f64 * 100.0
    }

    pub fn top3_pct(&self) -> f64 {
        self.top3 as f64 / self.questions.max(1) as f64 * 100.0
    }
}

fn read_questions(path: &Path) -> Result<Vec<Question>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("{} {}", tr("could not read", "no pude leer"), path.display()))?;
    Ok(toml::from_str::<EvalFile>(&text)?.q)
}

/// Runs every question through the chosen ranker. `weights` overrides the BM25 column weights
/// (used to tune them); it is ignored in fuzzy mode.
pub fn evaluate(
    path: &Path,
    mode: Mode,
    entries: &[Entry],
    db: &Db,
    weights: Option<Weights>,
) -> Result<Report> {
    let questions = read_questions(path)?;
    let mut fuzzy = (mode == Mode::Fuzzy).then(|| Searcher::new(entries));
    let mut params = Params { ..ask::PARAMS };
    if let Some(w) = weights {
        params.weights = w;
    }

    let (mut top1, mut top3, mut misses) = (0, 0, Vec::new());
    for q in &questions {
        let tools: Vec<String> = match fuzzy.as_mut() {
            Some(searcher) => searcher
                .rank(entries, &q.question, 3)
                .into_iter()
                .map(|i| entries[i].tool.clone())
                .collect(),
            None => ask::ask_with(db, &q.question, 3, &params)?
                .into_iter()
                .map(|c| c.entry.tool)
                .collect(),
        };
        let hit = |t: &String| q.tools.contains(t);
        if tools.first().is_some_and(hit) {
            top1 += 1;
        }
        if tools.iter().any(hit) {
            top3 += 1;
        } else {
            misses.push((q.question.clone(), tools.join(", ")));
        }
    }
    Ok(Report {
        questions: questions.len(),
        top1,
        top3,
        misses,
    })
}

pub fn run(path: &Path, mode: Mode, entries: &[Entry], db: &Db, weights: Option<Weights>) -> Result<()> {
    let report = evaluate(path, mode, entries, db, weights)?;
    println!("{} {}", tr("questions:", "preguntas:"), report.questions);
    println!("top-1:     {:.0}%", report.top1_pct());
    println!("top-3:     {:.0}%", report.top3_pct());
    if !report.misses.is_empty() {
        println!(
            "\n{}",
            tr("misses (what came out on top):", "fallos (lo que salió arriba):")
        );
        for (p, got) in &report.misses {
            println!(
                "  · {p}  →  {}",
                if got.is_empty() {
                    tr("nothing", "nada")
                } else {
                    got
                }
            );
        }
    }
    Ok(())
}
