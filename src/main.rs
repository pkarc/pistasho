mod ask;
mod config;
mod db;
mod eval;
mod history;
mod i18n;
mod index;
mod placeholders;
mod prompt;
mod search;
mod shell;
mod tldr;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use config::{Config, Height};
use i18n::tr;
use index::{Cheat, Index};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "pistasho",
    version,
    about = "The hint for the command you need, without leaving the terminal"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Opens the searcher and prints the chosen command
    Search {
        /// Initial search text
        #[arg(short, long, default_value = "")]
        query: String,
        /// Send the command to a tmux pane instead of printing it
        #[arg(long, value_name = "PANE")]
        send_to: Option<String>,
        /// Do not ask for values for the {{placeholders}}
        #[arg(long)]
        no_fill: bool,
        /// Height: `40%`, a number of lines or `full` (default: `ui.height`; the tmux popup is always full)
        #[arg(long)]
        height: Option<String>,
    },
    /// Search without the interface: prints the best results
    Query {
        query: Vec<String>,
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
        /// Print timings to stderr: index load, searcher build and per-keystroke ranking
        #[arg(long)]
        timing: bool,
    },
    /// Downloads the tldr pages and rebuilds the index
    Update,
    /// Adds a command to your cheatsheet
    Add {
        /// The command; use {{name}} for placeholders
        cmd: String,
        #[arg(short, long)]
        desc: String,
        #[arg(short, long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// Prints the integration code for your shell or tmux
    Init {
        #[arg(value_enum)]
        target: InitTarget,
    },
    /// Configuration wizard
    Config,
    /// Measures searcher accuracy over a set of questions
    Eval {
        #[arg(default_value = "eval/questions.toml")]
        file: PathBuf,
        /// Which ranker to measure
        #[arg(long, value_enum, default_value_t = eval::Mode::Bm25)]
        mode: eval::Mode,
        /// BM25 column weights for tuning: description,alt,tool,cmd (e.g. 3,1,5,1.5)
        #[arg(long, value_delimiter = ',')]
        weights: Option<Vec<f64>>,
    },
    /// Turns a question into the best command (prints only the command)
    Ask {
        question: Vec<String>,
        /// Print the ranked candidates as JSON, with their scores, instead of just the best one
        #[arg(long)]
        json: bool,
        /// How many candidates to include with --json
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
        /// Read the question on the prompt line (what the `Ctrl-Space` widget runs)
        #[arg(long)]
        prompt: bool,
        /// Text to pre-fill the question with (with --prompt)
        #[arg(long, default_value = "")]
        initial: String,
        /// How the result is written (with --prompt): `plain` is just the command; `zsh` is the
        /// command, a NUL byte and a one-line message for the widget
        #[arg(long, value_enum, default_value_t = OutputFormat::Plain)]
        format: OutputFormat,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum OutputFormat {
    Plain,
    Zsh,
}

#[derive(Clone, Copy, ValueEnum)]
enum InitTarget {
    Zsh,
    Tmux,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("pistasho: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse();
    // the interface language comes from the config; a broken config must not block the language
    i18n::init(&Config::load().map(|c| c.language).unwrap_or_default());
    match cli.command {
        Command::Search {
            query,
            send_to,
            no_fill,
            height,
        } => {
            let cfg = Config::load()?;
            let idx = Index::load(&cfg)?;
            if !idx.has_tldr && idx.entries.is_empty() {
                eprintln!(
                    "pistasho: {}",
                    tr(
                        "no index yet; run `pistasho config` or `pistasho update`",
                        "aún no hay índice; ejecuta `pistasho config` o `pistasho update`"
                    )
                );
                return Ok(ExitCode::from(2));
            }
            let outcome = tui::run(
                &idx.entries,
                tui::Options {
                    initial_query: query,
                    fill: !no_fill,
                    height: search_height(&cfg, height.as_deref(), send_to.is_some()),
                    start_fill: None,
                },
            )?;
            match outcome {
                tui::Outcome::Cancelled => Ok(ExitCode::from(130)),
                tui::Outcome::Selected(cmd) => {
                    match send_to {
                        Some(pane) => send_to_tmux(&pane, &cmd)?,
                        None => println!("{cmd}"),
                    }
                    Ok(ExitCode::SUCCESS)
                }
            }
        }
        Command::Query { query, limit, timing } => {
            let cfg = Config::load()?;
            let t0 = std::time::Instant::now();
            let idx = Index::load(&cfg)?;
            let t_load = t0.elapsed();
            let t1 = std::time::Instant::now();
            let mut s = search::Searcher::new(&idx.entries);
            let t_build = t1.elapsed();
            let text = query.join(" ");
            if timing {
                report_timing(&mut s, &idx.entries, &text, t_load, t_build);
            }
            for i in s.rank(&idx.entries, &text, limit) {
                let e = &idx.entries[i];
                println!("{:<5} {}\n      {}", e.source.tag(), e.cmd, e.desc);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Update => {
            let cfg = Config::load()?;
            let (pages, n) = index::update(&cfg)?;
            eprintln!(
                "✓ {pages} {}, {n} {} ({})",
                tr("pages", "páginas"),
                tr("examples", "ejemplos"),
                cfg.languages.join(", ")
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Add { cmd, desc, tags } => {
            let path = index::add_cheat(Cheat { cmd, desc, tags })?;
            eprintln!("✓ {} {}", tr("Added to", "Agregado a"), path.display());
            Ok(ExitCode::SUCCESS)
        }
        Command::Init { target } => {
            let cfg = Config::load()?;
            print!(
                "{}",
                match target {
                    InitTarget::Zsh => shell::zsh(&cfg),
                    InitTarget::Tmux => shell::tmux(&cfg),
                }
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Config => {
            let (cfg, download) = config::wizard()?;
            if download {
                let (pages, n) = index::update(&cfg)?;
                eprintln!(
                    "✓ {pages} {}, {n} {}",
                    tr("pages", "páginas"),
                    tr("examples", "ejemplos")
                );
            }
            println!(
                "\n{}\n  eval \"$(pistasho init zsh)\"",
                tr("Next step, in ~/.zshrc:", "Siguiente paso, en ~/.zshrc:")
            );
            println!(
                "{}\n  pistasho init tmux >> ~/.tmux.conf",
                tr("And for tmux:", "Y para tmux:")
            );
            Ok(ExitCode::SUCCESS)
        }
        Command::Eval { file, mode, weights } => {
            let cfg = Config::load()?;
            let idx = Index::load(&cfg)?;
            let db = index::open_synced(&cfg)?;
            let weights = match weights {
                None => None,
                Some(w) if w.len() == 4 => Some([w[0], w[1], w[2], w[3]]),
                Some(_) => anyhow::bail!(tr(
                    "--weights needs 4 comma-separated numbers: description,alt,tool,cmd",
                    "--weights necesita 4 números separados por comas: description,alt,tool,cmd"
                )),
            };
            eval::run(&file, mode, &idx.entries, &db, weights)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Ask {
            question,
            json,
            limit,
            prompt,
            initial,
            format,
        } => {
            let cfg = Config::load()?;
            if prompt {
                return ask_on_prompt(&cfg, &initial, format);
            }
            let db = index::open_synced(&cfg)?;
            let question = question.join(" ");
            let candidates = ask::ask(&db, &question, if json { limit } else { 1 })?;
            if json {
                let out = serde_json::json!({
                    "question": question,
                    "terms": ask::terms(&question),
                    "candidates": candidates.iter().map(|c| serde_json::json!({
                        "command": c.entry.cmd,
                        "description": c.entry.desc,
                        "tool": c.entry.tool,
                        "source": c.entry.source.db_name(),
                        "origin": c.entry.origin,
                        "score": c.score,
                    })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(ExitCode::SUCCESS);
            }
            match candidates.first() {
                Some(best) => {
                    println!("{}", best.entry.cmd);
                    Ok(ExitCode::SUCCESS)
                }
                None => {
                    eprintln!(
                        "pistasho: {}",
                        tr(
                            "no match for that question",
                            "ninguna coincidencia para esa pregunta"
                        )
                    );
                    Ok(ExitCode::from(1))
                }
            }
        }
    }
}

/// Diagnostic for `query --timing`: what the user waits for when opening the searcher
/// (index load + searcher build) and on each keystroke (one `rank` per typed prefix, the
/// same call the interface makes, with its 300-result limit).
fn report_timing(
    s: &mut search::Searcher,
    entries: &[index::Entry],
    text: &str,
    load: std::time::Duration,
    build: std::time::Duration,
) {
    let mut per_key = Vec::new();
    let mut prefix = String::new();
    for c in text.chars() {
        prefix.push(c);
        let t = std::time::Instant::now();
        let _ = s.rank(entries, &prefix, 300);
        per_key.push(t.elapsed());
    }
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    eprintln!("entries:        {}", entries.len());
    eprintln!("index load:     {:.1} ms", ms(load));
    eprintln!("searcher build: {:.1} ms", ms(build));
    eprintln!("open total:     {:.1} ms", ms(load + build));
    if !per_key.is_empty() {
        let max = per_key.iter().map(|d| ms(*d)).fold(0.0, f64::max);
        let avg = per_key.iter().map(|d| ms(*d)).sum::<f64>() / per_key.len() as f64;
        eprintln!(
            "per keystroke:  avg {avg:.1} ms, max {max:.1} ms ({} keys)",
            per_key.len()
        );
        let list: Vec<String> = per_key.iter().map(|d| format!("{:.0}", ms(*d))).collect();
        eprintln!("keystroke ms:   {}", list.join(" "));
    }
}

/// What `Ctrl-Space` runs: read the question on the prompt line, resolve it, ask for the
/// placeholders if the command has any, and hand the result to the widget.
///
/// Exit codes: 0 = a command is in the output, 1 = nothing matched (the output is the message),
/// 130 = cancelled.
fn ask_on_prompt(cfg: &Config, initial: &str, format: OutputFormat) -> Result<ExitCode> {
    let height = search_height(cfg, None, false);
    let (command, message) = match prompt::read_question(initial)? {
        prompt::Answer::Cancelled => return Ok(ExitCode::from(130)),
        prompt::Answer::Explore => {
            let idx = Index::load(cfg)?;
            let options = tui::Options {
                initial_query: String::new(),
                fill: true,
                height,
                start_fill: None,
            };
            match tui::run(&idx.entries, options)? {
                tui::Outcome::Cancelled => return Ok(ExitCode::from(130)),
                tui::Outcome::Selected(cmd) => (cmd, String::new()),
            }
        }
        prompt::Answer::Question(question) => {
            let started = std::time::Instant::now();
            let db = index::open_synced(cfg)?;
            let found = prompt::with_spinner(|| ask::ask(&db, &question, 1))??;
            let Some(best) = found.into_iter().next() else {
                let message = tr(
                    "✗ no match for that question",
                    "✗ ninguna coincidencia para esa pregunta",
                );
                match format {
                    OutputFormat::Zsh => print!("{message}"),
                    OutputFormat::Plain => eprintln!("pistasho: {message}"),
                }
                return Ok(ExitCode::from(1));
            };
            let message = format!(
                "⚡ local · {}/{} · {} ms",
                best.entry.source.tag(),
                best.entry.tool,
                started.elapsed().as_millis()
            );
            let command = if placeholders::has_placeholders(&best.entry.cmd) {
                let options = tui::Options {
                    initial_query: String::new(),
                    fill: true,
                    height,
                    start_fill: Some(best.entry.cmd.clone()),
                };
                match tui::run(&[], options)? {
                    tui::Outcome::Cancelled => return Ok(ExitCode::from(130)),
                    tui::Outcome::Selected(cmd) => cmd,
                }
            } else {
                best.entry.cmd
            };
            (command, message)
        }
    };
    match format {
        OutputFormat::Zsh => print!("{command}\0{message}"),
        OutputFormat::Plain => println!("{command}"),
    }
    Ok(ExitCode::SUCCESS)
}

/// The popup already bounds the searcher, so inside tmux it always uses its full area.
fn search_height(cfg: &Config, flag: Option<&str>, in_popup: bool) -> Height {
    if in_popup {
        return Height::Full;
    }
    flag.and_then(Height::parse)
        .or_else(|| Height::parse(&cfg.ui.height))
        .or_else(|| Height::parse(config::DEFAULT_HEIGHT))
        .unwrap_or(Height::Full)
}

fn send_to_tmux(pane: &str, cmd: &str) -> Result<()> {
    let status = std::process::Command::new("tmux")
        .args(["send-keys", "-t", pane, "-l", "--", cmd])
        .status()
        .context(tr("could not run tmux", "no pude ejecutar tmux"))?;
    if !status.success() {
        anyhow::bail!("tmux send-keys {} ({status})", tr("failed", "falló"));
    }
    Ok(())
}
