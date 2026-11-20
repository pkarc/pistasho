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
mod system;
mod tldr;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use config::{Config, Height};
use i18n::tr;
use index::{Cheat, Index};
use std::collections::HashMap;
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
        /// With --prompt: send the result to this tmux pane instead of printing it (the popup)
        #[arg(long, value_name = "PANE")]
        send_to: Option<String>,
        /// Open the list of alternatives for the question (what a second `Ctrl-Space` runs)
        #[arg(long)]
        alternatives: bool,
        /// Open the list of everything: your cheats, history and tldr
        #[arg(long)]
        explore: bool,
        /// The command now on the prompt, selected first in the list (with --alternatives)
        #[arg(long, default_value = "")]
        current: String,
        /// How the result is written (with --prompt, --alternatives, --explore): `plain` is just
        /// the command; `zsh` is `command NUL message NUL question`, for the widget
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
                    mode: tui::ListMode::Search,
                },
            )?;
            match outcome {
                tui::Outcome::Cancelled => Ok(ExitCode::from(130)),
                tui::Outcome::Selected(cmd, _origin) => {
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
            send_to,
            alternatives,
            explore,
            current,
            format,
        } => {
            let cfg = Config::load()?;
            if explore {
                return match explore_list(&cfg)? {
                    None => Ok(ExitCode::from(130)),
                    Some(cmd) => {
                        emit(format, &cmd, "", "");
                        Ok(ExitCode::SUCCESS)
                    }
                };
            }
            if alternatives {
                return ask_alternatives(&cfg, &question.join(" "), &current, format);
            }
            if prompt {
                return ask_on_prompt(&cfg, &initial, format, send_to.as_deref());
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
                        "available": c.missing.is_empty(),
                        "missing": c.missing,
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

/// What the widget gets on stdout with `--format zsh`: `command NUL message NUL question`.
/// The question travels back so the widget can reopen the alternatives for it.
fn emit(format: OutputFormat, command: &str, message: &str, question: &str) {
    match format {
        OutputFormat::Zsh => print!("{command}\0{message}\0{question}"),
        OutputFormat::Plain => println!("{command}"),
    }
}

/// Nothing matched: the output is the message, and the exit code is 1.
fn no_match(format: OutputFormat) -> ExitCode {
    let message = tr(
        "✗ no match for that question",
        "✗ ninguna coincidencia para esa pregunta",
    );
    match format {
        OutputFormat::Zsh => print!("{message}"),
        OutputFormat::Plain => eprintln!("pistasho: {message}"),
    }
    ExitCode::from(1)
}

/// What `Ctrl-Space` runs: read the question on the prompt line, resolve it, ask for the
/// placeholders if the command has any, and hand the result to the widget.
///
/// With `send_to` (the tmux popup) there is no prompt to put the answer on: the answer is shown
/// as the candidate list, best one selected (`⏎ send · ↑↓ see more`), and the pick is typed into
/// that pane without Enter.
///
/// Exit codes: 0 = a command is in the output (or was sent), 1 = nothing matched (the output is
/// the message), 130 = cancelled.
fn ask_on_prompt(
    cfg: &Config,
    initial: &str,
    format: OutputFormat,
    send_to: Option<&str>,
) -> Result<ExitCode> {
    let height = search_height(cfg, None, false);
    let (command, message, question) = match prompt::read_question(initial)? {
        prompt::Answer::Cancelled => return Ok(ExitCode::from(130)),
        prompt::Answer::Explore => match explore_list(cfg)? {
            None => return Ok(ExitCode::from(130)),
            Some(cmd) => (cmd, String::new(), String::new()),
        },
        prompt::Answer::Question(question) => {
            let started = std::time::Instant::now();
            let db = index::open_synced(cfg)?;
            let found = prompt::with_spinner(|| ask::ask(&db, &question, ALTERNATIVES))??;
            let more = found.len().saturating_sub(1);
            let Some(best) = found.into_iter().next() else {
                if send_to.is_some() {
                    // the popup closes with the process: keep the message on screen until a key
                    prompt::notice(tr(
                        "✗ no match for that question",
                        "✗ ninguna coincidencia para esa pregunta",
                    ))?;
                    return Ok(ExitCode::from(1));
                }
                return Ok(no_match(format));
            };
            let message = found_message(
                best.entry.source.tag(),
                &best.entry.tool,
                started.elapsed().as_millis(),
                more,
                &best.missing,
            );
            let command = if send_to.is_some() {
                match pick_alternative(cfg, &question, &best.entry.cmd, true)? {
                    None => return Ok(ExitCode::from(130)),
                    Some((cmd, _origin)) => cmd,
                }
            } else if placeholders::has_placeholders(&best.entry.cmd) {
                let options = tui::Options {
                    initial_query: String::new(),
                    fill: true,
                    height,
                    start_fill: Some(best.entry.cmd.clone()),
                    mode: tui::ListMode::Explore,
                };
                match tui::run(&[], options)? {
                    tui::Outcome::Cancelled => return Ok(ExitCode::from(130)),
                    tui::Outcome::Selected(cmd, _origin) => cmd,
                }
            } else {
                best.entry.cmd
            };
            (command, message, question)
        }
    };
    if let Some(pane) = send_to {
        send_to_tmux(pane, &command)?;
        return Ok(ExitCode::SUCCESS);
    }
    emit(format, &command, &message, &question);
    Ok(ExitCode::SUCCESS)
}

/// The list of everything (cheats, then history, then tldr) with a filter. `None` if cancelled.
fn explore_list(cfg: &Config) -> Result<Option<String>> {
    let idx = Index::load(cfg)?;
    let options = tui::Options {
        initial_query: String::new(),
        fill: true,
        height: search_height(cfg, None, false),
        start_fill: None,
        mode: tui::ListMode::Explore,
    };
    Ok(match tui::run(&idx.entries, options)? {
        tui::Outcome::Cancelled => None,
        tui::Outcome::Selected(cmd, _origin) => Some(cmd),
    })
}

/// The line under the prompt after an answer: where it came from, how long it took, and how to
/// get the other candidates.
fn found_message(source: &str, tool: &str, ms: u128, more: usize, missing: &[String]) -> String {
    let mut message = format!("⚡ local · {source}/{tool} · {ms} ms");
    if !missing.is_empty() {
        message.push_str(&format!(
            " · ⚠ {} {}",
            missing.join(", "),
            tr("not installed", "no instalado")
        ));
    }
    if more > 0 {
        let hint = match i18n::lang() {
            i18n::Lang::En => format!("Ctrl-Space: see {more} more"),
            i18n::Lang::Es => format!("Ctrl-Space: ver {more} más"),
        };
        message.push_str(" · ");
        message.push_str(&hint);
    }
    message
}

/// How many BM25 candidates the alternatives list starts with.
const ALTERNATIVES: usize = 20;

/// What a second `Ctrl-Space` runs: the candidates for the same question, with the command that
/// is on the prompt selected. Exit codes: 0 = replaced, 130 = kept the current one.
fn ask_alternatives(cfg: &Config, question: &str, current: &str, format: OutputFormat) -> Result<ExitCode> {
    match pick_alternative(cfg, question, current, false)? {
        None => Ok(ExitCode::from(130)),
        Some((cmd, origin)) => {
            let message = format!("✓ {} · {origin}", tr("replaced", "reemplazado"));
            emit(format, &cmd, &message, question);
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Shows the candidates for `question` with `current` selected, lets the user pick (filling in
/// placeholders if needed) and returns `(command, origin)`, or `None` if cancelled. With `send`
/// the hints say the pick is sent to a tmux pane rather than replacing the prompt.
fn pick_alternative(
    cfg: &Config,
    question: &str,
    current: &str,
    send: bool,
) -> Result<Option<(String, String)>> {
    let idx = Index::load(cfg)?;
    let db = index::open_synced(cfg)?;
    let found = ask::ask(&db, question, ALTERNATIVES)?;

    // the candidates come from the database, the list works on the loaded entries: link them
    let mut position: HashMap<(index::Source, &str), usize> = HashMap::new();
    for (i, e) in idx.entries.iter().enumerate() {
        position.entry((e.source, e.cmd.as_str())).or_insert(i);
    }
    let candidates: Vec<usize> = found
        .iter()
        .filter_map(|c| position.get(&(c.entry.source, c.entry.cmd.as_str())).copied())
        .collect();
    // the prompt holds the filled-in command; find the template it was made from
    let current_idx = candidates
        .iter()
        .copied()
        .find(|&i| placeholders::matches_template(&idx.entries[i].cmd, current));

    let options = tui::Options {
        initial_query: String::new(),
        fill: true,
        height: search_height(cfg, None, false),
        start_fill: None,
        mode: tui::ListMode::Alternatives {
            question: question.to_string(),
            candidates,
            current: current_idx,
            send,
        },
    };
    Ok(match tui::run(&idx.entries, options)? {
        tui::Outcome::Cancelled => None,
        tui::Outcome::Selected(cmd, origin) => Some((cmd, origin)),
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_message_points_to_the_other_candidates() {
        i18n::set(i18n::Lang::En);
        assert_eq!(
            found_message("tldr", "fc", 12, 5, &[]),
            "⚡ local · tldr/fc · 12 ms · Ctrl-Space: see 5 more"
        );
        // nothing else to show: no hint
        assert_eq!(
            found_message("mine", "fuser", 3, 0, &[]),
            "⚡ local · mine/fuser · 3 ms"
        );
    }

    #[test]
    fn answer_message_warns_about_programs_that_are_not_installed() {
        i18n::set(i18n::Lang::En);
        assert_eq!(
            found_message("tldr", "ncdu", 8, 4, &["ncdu".to_string()]),
            "⚡ local · tldr/ncdu · 8 ms · ⚠ ncdu not installed · Ctrl-Space: see 4 more"
        );
        assert!(found_message("tldr", "x", 1, 0, &["a".into(), "b".into()]).contains("⚠ a, b not installed"));
    }
}
