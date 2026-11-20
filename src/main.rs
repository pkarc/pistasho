mod config;
mod eval;
mod history;
mod i18n;
mod index;
mod placeholders;
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
    },
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
        Command::Eval { file } => {
            let cfg = Config::load()?;
            let idx = Index::load(&cfg)?;
            eval::run(&file, &idx.entries)?;
            Ok(ExitCode::SUCCESS)
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
