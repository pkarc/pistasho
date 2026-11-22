//! `pistasho eval`: measures how often the searcher puts the correct tool on top.
//!
//! Three rankers can be compared on the same questions: the fuzzy searcher of phase 1, the BM25
//! search, and BM25 fused with embeddings (what `pistasho ask` does when the model is there).

use crate::ask::{self, Fusion, Params};
use crate::db::{Db, Weights};
use crate::embed;
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
    /// BM25 fused with the embedding model by RRF (needs the model: `pistasho update`)
    Hybrid,
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
    /// Questions with a correct tool among the 10 best: the ceiling for anything that re-orders.
    pub recall10: usize,
    /// `(question, tools that came out on top)` for the questions with no hit in the top 3.
    pub misses: Vec<(String, String)>,
}

impl Report {
    pub fn top1_pct(&self) -> f64 {
        self.top1 as f64 / self.questions.max(1) as f64 * 100.0
    }

    pub fn recall10_pct(&self) -> f64 {
        self.recall10 as f64 / self.questions.max(1) as f64 * 100.0
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

/// A tool answers a question if it is the wanted one or one of its subcommand pages: `ip-address`
/// is `ip`, `git-diff` is `git`.
fn is_correct(tool: &str, wanted: &[String]) -> bool {
    wanted.iter().any(|w| {
        tool == w
            || tool
                .strip_prefix(w.as_str())
                .is_some_and(|rest| rest.starts_with('-'))
    })
}

/// Knobs for tuning, all optional.
#[derive(Default, Clone, Copy)]
pub struct Tuning {
    /// BM25 column weights (ignored in fuzzy mode).
    pub weights: Option<Weights>,
    pub fusion: Option<Fusion>,
    pub per_tool: Option<usize>,
    pub synonyms: Option<bool>,
}

/// Runs every question through the chosen ranker.
pub fn evaluate(path: &Path, mode: Mode, entries: &[Entry], db: &Db, tuning: Tuning) -> Result<Report> {
    let questions = read_questions(path)?;
    let mut fuzzy = (mode == Mode::Fuzzy).then(|| Searcher::new(entries));
    if mode == Mode::Hybrid && !embed::enabled() {
        anyhow::bail!(tr(
            "hybrid mode needs the embedding model: set search.embeddings = true and run `pistasho update`",
            "el modo hybrid necesita el modelo de embeddings: pon search.embeddings = true y corre `pistasho update`"
        ));
    }
    let mut params = Params { ..ask::PARAMS };
    if let Some(w) = tuning.weights {
        params.weights = w;
    }
    if let Some(f) = tuning.fusion {
        params.fusion = f;
    }
    if let Some(s) = tuning.synonyms {
        params.synonyms = s;
    }
    if let Some(n) = tuning.per_tool {
        params.per_tool = n;
    }

    let (mut top1, mut top3, mut recall10, mut misses) = (0, 0, 0, Vec::new());
    for q in &questions {
        let tools: Vec<String> = match fuzzy.as_mut() {
            Some(searcher) => searcher
                .rank(entries, &q.question, 10)
                .into_iter()
                .map(|i| entries[i].tool.clone())
                .collect(),
            None => {
                let qvec = (mode == Mode::Hybrid)
                    .then(|| embed::question_vector(&q.question))
                    .flatten();
                ask::ask_with(db, &q.question, 10, &params, qvec.as_deref())?
                    .into_iter()
                    .map(|c| c.entry.tool)
                    .collect()
            }
        };
        let hit = |t: &String| is_correct(t, &q.tools);
        if tools.first().is_some_and(hit) {
            top1 += 1;
        }
        if tools.iter().take(3).any(hit) {
            top3 += 1;
        } else {
            let shown: Vec<_> = tools.iter().take(3).cloned().collect();
            misses.push((q.question.clone(), shown.join(", ")));
        }
        if tools.iter().any(hit) {
            recall10 += 1;
        }
    }
    Ok(Report {
        questions: questions.len(),
        top1,
        top3,
        recall10,
        misses,
    })
}

pub fn run(path: &Path, mode: Mode, entries: &[Entry], db: &Db, tuning: Tuning) -> Result<()> {
    let report = evaluate(path, mode, entries, db, tuning)?;
    println!("{} {}", tr("questions:", "preguntas:"), report.questions);
    println!("top-1:     {:.0}%", report.top1_pct());
    println!("top-3:     {:.0}%", report.top3_pct());
    println!("recall@10: {:.0}%", report.recall10_pct());
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
