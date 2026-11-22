mod alternatives;
mod ask;
mod config;
mod db;
mod decide;
mod devices;
mod embed;
mod eval;
mod history;
mod i18n;
mod index;
mod llm;
mod packages;
mod placeholders;
mod prompt;
mod risk;
mod search;
mod semantic;
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
    /// Shows the OS, the package manager and what is needed to get the given programs
    Doctor {
        /// Programs to look up (none: only the OS and the package manager)
        programs: Vec<String>,
        /// Check every command of the built-in package table against the package manager
        #[arg(long)]
        table: bool,
    },
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
        /// Fusion for tuning: bm25 weight, meaning weight, RRF k (e.g. 1,2,60)
        #[arg(long, value_delimiter = ',')]
        fusion: Option<Vec<f64>>,
        /// Entries of one tool allowed before the others get their turn (0 = off)
        #[arg(long)]
        per_tool: Option<usize>,
        /// Do not add the synonyms of the question words (for A/B measurements)
        #[arg(long)]
        no_synonyms: bool,
        /// Also print how often the best candidate is right by score (to choose `llm.threshold`)
        #[arg(long)]
        confidence: bool,
        /// Let the decision model (`decide.provider = "jev"`) re-order the candidates: sends the eval
        /// questions and tldr candidates to the provider
        #[arg(long)]
        decide: bool,
    },
    /// What the fixed risk rules say about a command, as JSON (nothing is run)
    Risk {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// The LLM's cached answers: list, clear, or save one as a cheat
    Cache {
        #[command(subcommand)]
        action: CacheAction,
    },
    /// Turns a question into the best command (prints only the command)
    Ask {
        question: Vec<String>,
        /// Ask the configured LLM instead of searching locally (needs `[llm]` in the config)
        #[arg(long)]
        llm: bool,
        /// Print the ranked candidates as JSON, with their scores, instead of just the best one
        #[arg(long)]
        json: bool,
        /// Read one question per line from stdin and print one JSON line each (the candidates only):
        /// the index and the model load once, so thousands of questions take minutes (used to
        /// build training data, see training/)
        #[arg(long)]
        batch: bool,
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

#[derive(Subcommand)]
enum CacheAction {
    /// Shows what the LLM answered, newest first
    List,
    /// Forgets the answer to one question, or every answer
    Clear { question: Vec<String> },
    /// Saves the cached answer to a question in cheats.toml, so it is yours from now on
    Save { question: Vec<String> },
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
                    // the same checks as for an answer: a destructive command shows the red box
                    let Some(fin) = finalize(cmd, String::new(), None)? else {
                        return Ok(ExitCode::from(130));
                    };
                    if !fin.message.is_empty() {
                        eprintln!("pistasho: {}", fin.message);
                    }
                    match send_to {
                        Some(pane) => send_to_tmux(&pane, &fin.command)?,
                        None => println!("{}", fin.command),
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
        Command::Risk { command } => {
            let assessment = risk::assess(&command.join(" "));
            let level = match assessment.level {
                risk::Level::Ok => "ok",
                risk::Level::Caution => "caution",
                risk::Level::Destructive => "destructive",
            };
            let rules: Vec<&str> = assessment.findings.iter().map(|f| f.rule).collect();
            println!("{}", serde_json::json!({"level": level, "rules": rules}));
            Ok(ExitCode::SUCCESS)
        }
        Command::Cache { action } => cache_command(action),
        Command::Doctor { programs, table } => {
            doctor(&programs, table);
            Ok(ExitCode::SUCCESS)
        }
        Command::Eval {
            file,
            mode,
            weights,
            fusion,
            per_tool,
            no_synonyms,
            confidence,
            decide: use_decide,
        } => {
            let cfg = Config::load()?;
            embed::init(&cfg);
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
            let fusion = match fusion {
                None => None,
                Some(f) if f.len() == 3 => Some(ask::Fusion {
                    bm25: f[0],
                    meaning: f[1],
                    k: f[2],
                }),
                Some(_) => anyhow::bail!(tr(
                    "--fusion needs 3 comma-separated numbers: bm25 weight,meaning weight,k",
                    "--fusion necesita 3 números separados por comas: peso de bm25,peso de significado,k"
                )),
            };
            let jev = if use_decide {
                match decide::provider(&cfg) {
                    Some(j) => Some(j),
                    None => anyhow::bail!(tr(
                        "--decide needs decide.provider = \"jev\" in the config",
                        "--decide necesita decide.provider = \"jev\" en la config"
                    )),
                }
            } else {
                None
            };
            eval::run(
                &file,
                mode,
                &idx.entries,
                &db,
                eval::Tuning {
                    weights,
                    fusion,
                    per_tool,
                    synonyms: no_synonyms.then_some(false),
                },
                confidence,
                jev.as_ref(),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Ask {
            question,
            llm,
            batch,
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
            embed::init(&cfg);
            if !explore {
                // loads while the user types the question and the database opens
                embed::prefetch();
            }
            if explore {
                return match explore_list(&cfg)? {
                    None => Ok(ExitCode::from(130)),
                    Some(cmd) => {
                        let Some(fin) = finalize(cmd, String::new(), None)? else {
                            return Ok(ExitCode::from(130));
                        };
                        emit(format, &fin.command, &fin.message, "", false, fin.risk);
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
            if batch {
                return ask_batch(&cfg, limit);
            }
            let question = question.join(" ");
            if llm {
                return ask_llm_plain(&cfg, &question, None);
            }
            let db = index::open_synced(&cfg)?;
            let mut candidates = ask::ask(&db, &question, if json { limit } else { ALTERNATIVES })?;
            let mut jev_none = None;
            if !json {
                let decided = decide_found(&cfg, &question, candidates, false);
                candidates = decided.found;
                jev_none = decided.verdict.map(|v| v.none);
                if !decided.note.is_empty() {
                    eprintln!("pistasho: {}", decided.note);
                }
            }
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
                        "renames": c.renames,
                        "install": c.missing.first().and_then(|bin| install_json(bin)),
                        "alternative": alternatives::alternative_for(&c.entry.cmd, &c.missing),
                    })).collect::<Vec<_>>(),
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
                return Ok(ExitCode::SUCCESS);
            }
            if !json
                && (llm::wanted(&cfg, candidates.first(), jev_none) || llm::unknown_provider(&cfg).is_some())
            {
                // a failed LLM falls back to the local best, if there is one
                return ask_llm_plain(&cfg, &question, candidates.first().map(|c| c.entry.cmd.as_str()));
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

/// What the widget gets on stdout with `--format zsh`: `command NUL message NUL question NUL
/// printed NUL risk NUL label`. The question travels back so the widget can reopen the alternatives for it, and
/// `printed` is `1` when something other than pistasho's own interface wrote on the terminal
/// (the package manager), which tells zle to redraw the prompt from the line it is now on.
fn emit(
    format: OutputFormat,
    command: &str,
    message: &str,
    question: &str,
    printed: bool,
    risk: risk::Level,
) {
    match format {
        OutputFormat::Zsh => print!(
            "{command}\0{message}\0{question}\0{}\0{}\0{}",
            u8::from(printed),
            risk.name(),
            risk.label()
        ),
        OutputFormat::Plain => println!("{command}"),
    }
}

/// Nothing matched: the output is the message, and the exit code is 1.
/// `pistasho ask --batch`: a question per line in, a JSON line of candidates per question out.
fn ask_batch(cfg: &Config, limit: usize) -> Result<ExitCode> {
    use std::io::{BufRead, Write};
    let db = index::open_synced(cfg)?;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let question = line?;
        let candidates = if question.trim().is_empty() {
            Vec::new()
        } else {
            ask::ask(&db, &question, limit)?
        };
        let json = serde_json::json!({
            "question": question,
            "candidates": candidates.iter().map(|c| serde_json::json!({
                "command": c.entry.cmd,
                "description": c.entry.desc,
                "tool": c.entry.tool,
                "source": c.entry.source.db_name(),
                "score": c.score,
            })).collect::<Vec<_>>(),
        });
        writeln!(out, "{json}")?;
    }
    Ok(ExitCode::SUCCESS)
}

/// `pistasho ask` without the interface: the LLM's command on stdout, how it was made on stderr.
fn ask_llm_plain(cfg: &Config, question: &str, local: Option<&str>) -> Result<ExitCode> {
    let db = index::open_synced(cfg)?;
    match llm_resolve(cfg, &db, question, false) {
        LlmOutcome::Command(command, message) => {
            println!("{command}");
            eprintln!("{message}");
            Ok(ExitCode::SUCCESS)
        }
        LlmOutcome::Failed(message) => {
            eprintln!("pistasho: {message}");
            match local {
                Some(command) => {
                    println!("{command}");
                    Ok(ExitCode::SUCCESS)
                }
                None => Ok(ExitCode::from(1)),
            }
        }
        LlmOutcome::Off => {
            if let Some(command) = local {
                println!("{command}");
                return Ok(ExitCode::SUCCESS);
            }
            eprintln!(
                "pistasho: {}",
                tr(
                    "the LLM is off: set llm.provider in the config (pistasho config)",
                    "el LLM está apagado: pon llm.provider en la config (pistasho config)"
                )
            );
            Ok(ExitCode::from(1))
        }
    }
}

/// What the LLM fallback came to.
enum LlmOutcome {
    /// Not configured: carry on with "no match".
    Off,
    /// A command and the line to show under the prompt.
    Command(String, String),
    /// It was asked and failed; the text says why.
    Failed(String),
}

/// Asks the configured LLM (or finds what it answered before), behind the spinner if there is
/// a screen. Nothing is sent unless `llm.provider` is set.
fn llm_resolve(cfg: &Config, db: &db::Db, question: &str, spinner: bool) -> LlmOutcome {
    if let Some(name) = llm::unknown_provider(cfg) {
        return LlmOutcome::Failed(format!(
            "✗ llm: {} “{name}” (anthropic)",
            tr("unknown provider", "proveedor desconocido")
        ));
    }
    if llm::provider(cfg).is_none() {
        return LlmOutcome::Off;
    }
    let answer = if spinner {
        prompt::with_spinner(|| llm::resolve(cfg, db, question)).and_then(|r| r)
    } else {
        llm::resolve(cfg, db, question)
    };
    match answer {
        Ok(Some(answer)) => LlmOutcome::Command(answer.suggestion.command.clone(), llm::message(&answer)),
        Ok(None) => LlmOutcome::Off,
        Err(e) => LlmOutcome::Failed(format!("✗ llm: {e}")),
    }
}

/// The local candidates after the decision model has had a look.
struct Decided {
    found: Vec<ask::Candidate>,
    /// What it made of them; `None` when it is off, not needed (the best answer is yours) or failed.
    verdict: Option<decide::Verdict>,
    /// Why it did not answer, for the line under the prompt.
    note: String,
}

/// Lets the decision model re-order the tldr candidates, behind the spinner if there is a screen.
/// Failing is not an error: the local order stays.
fn decide_found(cfg: &Config, question: &str, found: Vec<ask::Candidate>, spinner: bool) -> Decided {
    let off = |found| Decided {
        found,
        verdict: None,
        note: String::new(),
    };
    if let Some(name) = decide::unknown_provider(cfg) {
        return Decided {
            note: format!(
                "✗ decide: {} “{name}” (jev)",
                tr("unknown provider", "proveedor desconocido")
            ),
            ..off(found)
        };
    }
    let Some(jev) = decide::provider(cfg) else {
        return off(found);
    };
    if found
        .first()
        .map_or(true, |c| c.entry.source != index::Source::Tldr)
    {
        return off(found);
    }
    // `apply` consumes the candidates: keep a copy to fall back on if the call fails
    let backup = found.clone();
    let result = if spinner {
        prompt::with_spinner(|| decide::apply(&jev, question, found)).and_then(|r| r)
    } else {
        decide::apply(&jev, question, found)
    };
    match result {
        Ok((found, verdict)) => Decided {
            found,
            verdict,
            note: String::new(),
        },
        Err(e) => Decided {
            note: format!("✗ decide: {e}"),
            ..off(backup)
        },
    }
}

/// The decision model's opinion on how destructive a command template is (never a command with
/// your values in it). `None` when it is off, the rules already say destructive, or it failed:
/// the rules still apply either way.
fn decide_risk(cfg: &Config, template: &str, spinner: bool) -> Option<f64> {
    if !cfg.decide.risk || risk::assess(template).level == risk::Level::Destructive {
        return None;
    }
    let jev = decide::provider(cfg)?;
    let result = if spinner {
        prompt::with_spinner(|| jev.destructive(template)).and_then(|r| r)
    } else {
        jev.destructive(template)
    };
    result.ok()
}

/// `· Ctrl-Space: see N more`, when there are other candidates to see.
fn with_more_hint(mut message: String, more: usize) -> String {
    if more > 0 {
        message.push_str(" · ");
        message.push_str(&more_hint(more));
    }
    message
}

fn more_hint(more: usize) -> String {
    match i18n::lang() {
        i18n::Lang::En => format!("Ctrl-Space: see {more} more"),
        i18n::Lang::Es => format!("Ctrl-Space: ver {more} más"),
    }
}

/// Reports a failure the way "no match" is reported (a popup keeps it on screen until a key).
fn say_failure(format: OutputFormat, message: &str, popup: bool) -> ExitCode {
    if popup {
        let _ = prompt::notice(message);
        return ExitCode::from(1);
    }
    match format {
        OutputFormat::Zsh => print!("{message}"),
        OutputFormat::Plain => eprintln!("pistasho: {message}"),
    }
    ExitCode::from(1)
}

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

/// The install ran and failed (or did not leave the program in place). The output is the message,
/// and the terminal has been written to, so the widget redraws from the current line.
const EXIT_INSTALL_FAILED: u8 = 5;

/// What came of an answer that needs programs that are not installed.
enum Install {
    /// Nothing to install, offers are off, or no package provides it: carry on with the command
    /// as it is (the message under the prompt warns about what is missing).
    NotNeeded,
    /// Installed just now. The text says so for the message under the prompt.
    Done(String),
    /// The user chose the alternative: this command replaces the answer, nothing is installed.
    Alternative(String),
    /// The user declined in the box.
    Cancelled,
    /// The install ran and failed, or the program is still not there afterwards.
    Failed(String),
}

/// If the command needs programs that are not installed and a package provides them, shows the
/// confirmation box (`[y]` install, `[esc]` cancel) and, only on an explicit yes, runs the
/// install in the foreground on the terminal. Then checks again that the programs are there.
///
/// This is the one thing pistasho runs by itself: always after confirmation, with the exact
/// command on screen, and never through a shell.
fn offer_install(cfg: &Config, command: &str) -> Result<Install> {
    if !cfg.install.offer {
        return Ok(Install::NotNeeded);
    }
    let still_missing = || system::missing_binaries(&packages::adapt(command).command);
    let missing = still_missing();
    if missing.is_empty() {
        return Ok(Install::NotNeeded);
    }
    let plan = packages::plan_install_for(&missing);
    let alternative = alternatives::alternative_for(command, &missing);
    if plan.is_none() && alternative.is_none() {
        // nothing to offer: the message under the prompt warns about what is missing
        return Ok(Install::NotNeeded);
    }
    let manager = plan
        .as_ref()
        .map(|p| p.manager)
        .or_else(packages::Manager::detect);
    let system_line = match (packages::detect_os(), manager) {
        (Some(os), Some(m)) if !os.pretty_name.is_empty() => format!("{} · {}", os.pretty_name, m.name()),
        (Some(os), None) if !os.pretty_name.is_empty() => os.pretty_name,
        (_, Some(m)) => m.name().to_string(),
        _ => tr("unknown system", "sistema desconocido").to_string(),
    };
    let info = tui::InstallBox {
        command: packages::adapt(command).command,
        system: system_line,
        install: plan.as_ref().map(|p| p.display()),
        missing: missing.clone(),
        alternative,
    };
    let plan = match tui::confirm_install(&info)? {
        tui::InstallChoice::Cancel => return Ok(Install::Cancelled),
        tui::InstallChoice::Alternative => {
            return Ok(info.alternative.map_or(Install::NotNeeded, Install::Alternative));
        }
        // the box only lets `y` through when there is a plan
        tui::InstallChoice::Install => match plan {
            Some(plan) => plan,
            None => return Ok(Install::NotNeeded),
        },
    };

    prompt::say(&format!("$ {}", plan.display()))?;
    let failed = tr("✗ installation failed", "✗ falló la instalación");
    match packages::run_install(&plan) {
        Err(e) => return Ok(Install::Failed(format!("{failed}: {e}"))),
        Ok(false) => return Ok(Install::Failed(failed.to_string())),
        Ok(true) => {}
    }
    let left = still_missing();
    if !left.is_empty() {
        return Ok(Install::Failed(format!(
            "✗ {} {}",
            left.join(", "),
            tr(
                "is still not found after installing",
                "sigue sin aparecer tras instalar"
            )
        )));
    }
    Ok(Install::Done(format!(
        "✓ {} {}",
        plan.packages.join(", "),
        tr("installed", "instalado")
    )))
}

/// What `Ctrl-Space` runs: read the question on the prompt line, resolve it, offer to install
/// what it needs, ask for the placeholders if the command has any, and hand the result to the
/// widget.
///
/// With `send_to` (the tmux popup) there is no prompt to put the answer on: the answer is shown
/// as the candidate list, best one selected (`⏎ send · ↑↓ see more`), and the pick is typed into
/// that pane without Enter.
///
/// Exit codes: 0 = a command is in the output (or was sent), 1 = nothing matched (the output is
/// the message), 5 = the install failed (the output is the message), 130 = cancelled.
fn ask_on_prompt(
    cfg: &Config,
    initial: &str,
    format: OutputFormat,
    send_to: Option<&str>,
) -> Result<ExitCode> {
    let height = search_height(cfg, None, false);
    let mut printed = false;
    let mut installed_note = String::new();
    let (command, message, question, model_risk) = match prompt::read_question(initial)? {
        prompt::Answer::Cancelled => return Ok(ExitCode::from(130)),
        prompt::Answer::Explore => match explore_list(cfg)? {
            None => return Ok(ExitCode::from(130)),
            Some(cmd) => (cmd, String::new(), String::new(), None),
        },
        prompt::Answer::Question(question) => {
            let started = std::time::Instant::now();
            let db = index::open_synced(cfg)?;
            let found = prompt::with_spinner(|| ask::ask(&db, &question, ALTERNATIVES))??;
            // the decision model (if on) re-orders the tldr candidates and says when none fits
            let decided = decide_found(cfg, &question, found, true);
            let (found, verdict) = (decided.found, decided.verdict);
            // a service that just failed is not asked again for the risk (it would only wait twice)
            let decide_failed = !decided.note.is_empty();
            let more = found.len().saturating_sub(1);
            // the LLM (or what it answered before) takes over when nothing local matches or the
            // best match is shaky; its failure only matters if there is nothing local to show
            let mut llm_note = String::new();
            let mut from_llm = None;
            if llm::wanted(cfg, found.first(), verdict.as_ref().map(|v| v.none))
                || llm::unknown_provider(cfg).is_some()
            {
                match llm_resolve(cfg, &db, &question, true) {
                    LlmOutcome::Command(command, message) => from_llm = Some((command, message)),
                    LlmOutcome::Failed(message) if found.is_empty() => {
                        return Ok(say_failure(format, &message, send_to.is_some()))
                    }
                    LlmOutcome::Failed(message) => llm_note = message,
                    LlmOutcome::Off => {}
                }
            }
            // how long resolving took, taken now: the box below waits for the user
            let resolved_ms = started.elapsed().as_millis();
            // what is offered: its command, the line under the prompt, programs it lacks
            let (offered, mut message, missing, is_llm, public) = match (from_llm, found.first()) {
                (Some((command, message)), _) => {
                    // the local candidates are still the alternatives of a second Ctrl-Space
                    let message = with_more_hint(message, found.len());
                    let missing = system::missing_binaries(&packages::adapt(&command).command);
                    (command, message, missing, true, true)
                }
                (None, Some(best)) => {
                    let mut message =
                        found_message(best.entry.source.tag(), &best.entry.tool, resolved_ms, more);
                    // the model's confidence, when it picked this one (not when it said "none")
                    if let Some(v) = verdict.as_ref().filter(|v| !v.none) {
                        message = format!("{message} · ✦ jev {:.0}%", v.confidence * 100.0);
                    }
                    for note in [&decided.note, &llm_note] {
                        if !note.is_empty() {
                            message = format!("{message} · {note}");
                        }
                    }
                    let public = best.entry.source == index::Source::Tldr;
                    (
                        best.entry.cmd.clone(),
                        message,
                        best.missing.clone(),
                        false,
                        public,
                    )
                }
                (None, None) => {
                    if send_to.is_some() {
                        // the popup closes with the process: keep the message on screen until a key
                        prompt::notice(tr(
                            "✗ no match for that question",
                            "✗ ninguna coincidencia para esa pregunta",
                        ))?;
                        return Ok(ExitCode::from(1));
                    }
                    return Ok(no_match(format));
                }
            };
            let mut alternative: Option<String> = None;
            if !missing.is_empty() {
                match offer_install(cfg, &offered)? {
                    Install::NotNeeded => {}
                    Install::Cancelled => return Ok(ExitCode::from(130)),
                    Install::Alternative(command) => alternative = Some(command),
                    Install::Done(note) => {
                        printed = true;
                        if send_to.is_some() {
                            // the popup closes with the process: let the output be read first
                            prompt::notice(&note)?;
                        }
                        installed_note = note;
                    }
                    Install::Failed(message) => {
                        if send_to.is_some() {
                            prompt::notice(&message)?;
                            return Ok(ExitCode::from(1));
                        }
                        match format {
                            OutputFormat::Zsh => print!("{message}"),
                            OutputFormat::Plain => eprintln!("pistasho: {message}"),
                        }
                        return Ok(ExitCode::from(EXIT_INSTALL_FAILED));
                    }
                }
            }
            // the alternative stands in for the answer: same steps, its own command
            let chosen_alternative = alternative.is_some();
            if chosen_alternative {
                message = format!(
                    "⚡ {} · {} · {} ms",
                    tr("alternative", "alternativa"),
                    tr("no install", "sin instalar"),
                    resolved_ms
                );
            }
            let template = alternative.unwrap_or_else(|| offered.clone());
            // the decision model also gives an opinion on the risk, of the template only: your own
            // commands and the values you fill in never go anywhere
            let model_risk = if (public || chosen_alternative) && !decide_failed {
                decide_risk(cfg, &template, true)
            } else {
                None
            };
            // the popup lists the local candidates to pick from; an LLM answer is not one of them
            let command = if send_to.is_some() && !chosen_alternative && !is_llm {
                match pick_alternative(cfg, &question, &offered, true)? {
                    None => return Ok(ExitCode::from(130)),
                    Some((cmd, _origin)) => cmd,
                }
            } else if placeholders::has_placeholders(&template) {
                let options = tui::Options {
                    initial_query: String::new(),
                    fill: true,
                    height,
                    start_fill: Some(template),
                    mode: tui::ListMode::Explore,
                };
                match tui::run(&[], options)? {
                    tui::Outcome::Cancelled => return Ok(ExitCode::from(130)),
                    tui::Outcome::Selected(cmd, _origin) => cmd,
                }
            } else {
                template
            };
            (command, message, question, model_risk)
        }
    };
    let Some(fin) = finalize(command, message, model_risk)? else {
        return Ok(ExitCode::from(130));
    };
    let mut message = fin.message;
    if !installed_note.is_empty() {
        message.push_str(" · ");
        message.push_str(&installed_note);
    }
    if let Some(pane) = send_to {
        send_to_tmux(pane, &fin.command)?;
        return Ok(ExitCode::SUCCESS);
    }
    emit(format, &fin.command, &message, &question, printed, fin.risk);
    Ok(ExitCode::SUCCESS)
}

/// The command as it is handed over, the line under the prompt, and how dangerous it is.
struct Final {
    command: String,
    message: String,
    risk: risk::Level,
}

/// What a command gets before it is handed over (typed on the prompt or sent to a pane):
/// programs this machine has under another name are put right (`fd` → `fdfind`), the risk rules
/// look at it, and a destructive one is shown in the red box first, where the user can leave it
/// on the prompt, edit it, or drop it. `None` means dropped.
///
/// Nothing is run: leaving a command on the prompt is writing text.
fn finalize(command: String, message: String, model_risk: Option<f64>) -> Result<Option<Final>> {
    let (mut command, mut model_risk) = (command, model_risk);
    loop {
        let adapted = packages::adapt(&command);
        let mut assessment = risk::assess(&adapted.command);
        if let Some(p) = model_risk {
            assessment = risk::with_model(assessment, p);
        }
        if assessment.level == risk::Level::Destructive {
            match confirm_destructive(&adapted.command, &assessment)? {
                tui::RiskChoice::Leave => {}
                tui::RiskChoice::Cancel => return Ok(None),
                tui::RiskChoice::Edit => {
                    // edited text is checked again from the top; giving up goes back to the box
                    if let Some(edited) = prompt::edit_command(&adapted.command)? {
                        command = edited;
                        // the model judged the template, not what the user typed
                        model_risk = None;
                    }
                    continue;
                }
            }
        }
        let missing = system::missing_binaries(&adapted.command);
        let message = final_message(message, &adapted.renames, &missing, &assessment);
        return Ok(Some(Final {
            command: adapted.command,
            message,
            risk: assessment.level,
        }));
    }
}

/// The line under the prompt: what was said so far, then renames, programs still missing, and
/// the risk.
fn final_message(
    mut message: String,
    renames: &[(String, String)],
    missing: &[String],
    assessment: &risk::Assessment,
) -> String {
    let mut push = |text: String| {
        if !message.is_empty() {
            message.push_str(" · ");
        }
        message.push_str(&text);
    };
    for (asked, installed) in renames {
        push(format!("{asked} → {installed}"));
    }
    if !missing.is_empty() {
        push(format!(
            "⚠ {} {}",
            missing.join(", "),
            tr("not installed", "no instalado")
        ));
    }
    match assessment.level {
        risk::Level::Ok => {}
        risk::Level::Caution => push(format!(
            "⚠ {} · {}",
            assessment.level.label(),
            assessment.effect()
        )),
        risk::Level::Destructive => push(format!(
            "⚠ {} · {}",
            assessment.level.label(),
            tr(
                "review before running — it never runs by itself",
                "revisa antes de ejecutar — nunca se ejecuta solo"
            )
        )),
    }
    message
}

/// Shows the red box for a destructive command: what it does, what device it writes to (from
/// `lsblk`) and which fixed rule caught it.
fn confirm_destructive(command: &str, assessment: &risk::Assessment) -> Result<tui::RiskChoice> {
    let known = if assessment.devices.is_empty() {
        Vec::new()
    } else {
        devices::lsblk().unwrap_or_default()
    };
    let device = assessment
        .devices
        .first()
        .map(|path| describe_device(&known, path));
    tui::confirm_risk(&tui::RiskBox {
        command: command.to_string(),
        effect: assessment.effect(),
        device,
        signals: assessment.rules(),
        can_edit: !command.contains('\n'),
    })
}

/// What a device in a command is, for the red box: its model, size, filesystem and where it is
/// mounted, plus a warning when it is not removable (a system disk).
fn describe_device(known: &[devices::Device], path: &str) -> String {
    match devices::find(known, path) {
        Some(d) => {
            let mut text = d.describe();
            if let Some(mount) = &d.mountpoint {
                text = text.replace(
                    &format!("  {mount}"),
                    &format!(" · {} {mount}", tr("mounted at", "montado en")),
                );
            }
            if !d.removable {
                text.push_str(&format!(
                    " · {}",
                    tr(
                        "NOT removable: this looks like a system disk",
                        "NO es extraíble: parece un disco del sistema"
                    )
                ));
            }
            text
        }
        None => format!(
            "{path} ({})",
            tr("not found on this machine", "no existe en esta máquina")
        ),
    }
}

/// The install plan for a program that is not installed, as JSON for `ask --json`.
fn install_json(bin: &str) -> Option<serde_json::Value> {
    let plan = packages::plan_install(bin)?;
    Some(serde_json::json!({
        "manager": plan.manager.name(),
        "packages": plan.packages,
        "command": plan.display(),
    }))
}

/// `pistasho doctor`: what this machine is and what it would take to get a program.
/// Whether the decision model is on and ready; never shows the key, only whether it is there.
fn decide_line() -> String {
    let cfg = Config::load().unwrap_or_default();
    if let Some(name) = decide::unknown_provider(&cfg) {
        return format!(
            "{} “{name}” (jev)",
            tr("unknown provider", "proveedor desconocido")
        );
    }
    if cfg.decide.provider != "jev" {
        return tr(
            "off (decide.provider = \"none\")",
            "apagado (decide.provider = \"none\")",
        )
        .into();
    }
    let var = cfg
        .decide
        .api_key_env
        .clone()
        .unwrap_or_else(|| decide::DEFAULT_KEY_ENV.into());
    let risk = if cfg.decide.risk {
        tr("choice + risk", "elección + riesgo")
    } else {
        tr("choice only", "solo elección")
    };
    if std::env::var(&var).is_ok_and(|k| !k.trim().is_empty()) {
        format!("jev · {risk}")
    } else {
        format!("jev · {risk} · ✗ {} {var}", tr("set", "define"))
    }
}

/// Whether the LLM fallback is on and ready; never shows the key, only whether it is there.
fn llm_line() -> String {
    let cfg = Config::load().unwrap_or_default();
    if let Some(name) = llm::unknown_provider(&cfg) {
        return format!("{} “{name}”", tr("unknown provider", "proveedor desconocido"));
    }
    if cfg.llm.provider != "anthropic" {
        return tr(
            "off (llm.provider = \"none\")",
            "apagado (llm.provider = \"none\")",
        )
        .into();
    }
    let var = cfg
        .llm
        .api_key_env
        .clone()
        .unwrap_or_else(|| llm::DEFAULT_KEY_ENV.into());
    let key = std::env::var(&var).is_ok_and(|k| !k.trim().is_empty());
    let model = cfg.llm.model.clone().unwrap_or_else(|| llm::DEFAULT_MODEL.into());
    let when = if cfg.llm.when == "nomatch" {
        tr("only when nothing matches", "solo si nada coincide").to_string()
    } else {
        format!(
            "{} < {}",
            tr("when the best score is", "cuando el mejor puntaje es"),
            cfg.llm.threshold.unwrap_or(llm::DEFAULT_THRESHOLD)
        )
    };
    if key {
        format!("anthropic · {model} · {when}")
    } else {
        format!("anthropic · {model} · ✗ {} {var}", tr("set", "define"))
    }
}

/// Whether `ask` understands questions by meaning, and if not, why.
fn embeddings_line() -> String {
    let cfg = Config::load().unwrap_or_default();
    if !cfg.search.embeddings {
        return tr(
            "off (search.embeddings = false)",
            "apagado (search.embeddings = false)",
        )
        .into();
    }
    if !embed::model_present() {
        return tr(
            "no model yet: run `pistasho update`",
            "sin modelo todavía: corre `pistasho update`",
        )
        .into();
    }
    let coverage = config::db_path()
        .ok()
        .and_then(|p| db::Db::open(&p).ok())
        .and_then(|db| db.vector_coverage().ok());
    match coverage {
        Some((all, have)) if have >= all && all > 0 => {
            format!("{} ({have} {})", tr("on", "activo"), tr("vectors", "vectores"))
        }
        Some((all, have)) => format!(
            "{} {have}/{all}: {}",
            tr("incomplete,", "incompleto,"),
            tr("run `pistasho update`", "corre `pistasho update`")
        ),
        None => tr("on", "activo").into(),
    }
}

/// `pistasho cache`: look at, drop or keep the LLM's answers.
fn cache_command(action: CacheAction) -> Result<ExitCode> {
    let db = db::Db::open(&config::db_path()?)?;
    match action {
        CacheAction::List => {
            let all = db.cache_list()?;
            if all.is_empty() {
                eprintln!("{}", tr("nothing cached yet", "no hay nada en la caché todavía"));
            }
            for a in all {
                println!("{}\n  → {}\n  ({}, {})", a.question, a.command, a.model, a.risk);
            }
            Ok(ExitCode::SUCCESS)
        }
        CacheAction::Clear { question } => {
            let key = ask::normalize_question(&question.join(" "));
            let n = db.cache_clear((!key.is_empty()).then_some(key.as_str()))?;
            eprintln!("✓ {n} {}", tr("forgotten", "olvidadas"));
            Ok(ExitCode::SUCCESS)
        }
        CacheAction::Save { question } => {
            let text = question.join(" ");
            let Some(answer) = db.cache_get(&ask::normalize_question(&text))? else {
                eprintln!(
                    "pistasho: {}",
                    tr(
                        "no cached answer for that question (pistasho cache list)",
                        "no hay respuesta en caché para esa pregunta (pistasho cache list)"
                    )
                );
                return Ok(ExitCode::from(1));
            };
            let path = index::add_cheat(Cheat {
                cmd: answer.command.clone(),
                desc: text,
                tags: vec!["llm".into()],
            })?;
            eprintln!("✓ {} {}", tr("Added to", "Agregado a"), path.display());
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn doctor(programs: &[String], table: bool) {
    let os = packages::detect_os();
    let manager = packages::Manager::detect();
    let os_line = match &os {
        Some(o) if !o.pretty_name.is_empty() => format!("{} ({})", o.pretty_name, o.id),
        Some(o) => o.id.clone(),
        None => tr("unknown", "desconocido").to_string(),
    };
    println!("{:<9} {os_line}", tr("OS:", "SO:"));
    println!(
        "{:<9} {}",
        tr("Manager:", "Gestor:"),
        manager.map_or(tr("none known", "ninguno conocido"), |m| m.name())
    );
    if !table {
        println!("{:<9} {}", tr("Meaning:", "Significado:"), embeddings_line());
        println!("{:<9} {}", "LLM:", llm_line());
        println!("{:<9} {}", tr("Decision:", "Decisión:"), decide_line());
    }
    if table {
        let Some(manager) = manager else { return };
        for command in packages::table_commands() {
            let package = packages::package_for(&command, manager);
            let ok = packages::exists(manager, &package).unwrap_or(false);
            println!("{} {command} → {package}", if ok { "OK     " } else { "MISSING" });
        }
        return;
    }
    for bin in programs {
        println!("{bin:<14} {}", describe_program(bin, manager));
    }
}

/// One line about a program: installed, a builtin, installed under another name, or how to get it.
fn describe_program(bin: &str, manager: Option<packages::Manager>) -> String {
    if system::is_builtin(bin) {
        return tr("shell builtin", "comando del shell").to_string();
    }
    if let Some(path) = system::which(bin) {
        return format!("{} ({})", tr("installed", "instalado"), path.display());
    }
    let Some(manager) = manager else {
        return tr(
            "not installed; no package manager known",
            "no instalado; sin gestor de paquetes conocido",
        )
        .to_string();
    };
    if let Some(other) = packages::installed_name(bin, manager) {
        if system::which(other).is_some() {
            return format!(
                "{} {other}",
                tr("installed under another name:", "instalado con otro nombre:")
            );
        }
    }
    match packages::plan_install(bin) {
        Some(plan) => format!(
            "{} · {} {} · {}",
            tr("not installed", "no instalado"),
            tr("package", "paquete"),
            plan.packages.join(" "),
            plan.display()
        ),
        None => format!(
            "{} · {} ({})",
            tr("not installed", "no instalado"),
            tr("no package found", "no se encontró paquete"),
            manager.name()
        ),
    }
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
fn found_message(source: &str, tool: &str, ms: u128, more: usize) -> String {
    let message = format!("⚡ local · {source}/{tool} · {ms} ms");
    with_more_hint(message, more)
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
            let Some(fin) = finalize(cmd, message, None)? else {
                return Ok(ExitCode::from(130));
            };
            emit(format, &fin.command, &fin.message, question, false, fin.risk);
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
            found_message("tldr", "fc", 12, 5),
            "⚡ local · tldr/fc · 12 ms · Ctrl-Space: see 5 more"
        );
        // nothing else to show: no hint
        assert_eq!(
            found_message("mine", "fuser", 3, 0),
            "⚡ local · mine/fuser · 3 ms"
        );
    }

    #[test]
    fn the_final_message_collects_renames_missing_programs_and_risk() {
        i18n::set(i18n::Lang::En);
        let ok = risk::assess("ls");
        assert_eq!(final_message("⚡ local".into(), &[], &[], &ok), "⚡ local");
        assert_eq!(
            final_message(String::new(), &[], &["x".into(), "y".into()], &ok),
            "⚠ x, y not installed"
        );
        assert_eq!(
            final_message("⚡".into(), &[("fd".into(), "fdfind".into())], &[], &ok),
            "⚡ · fd → fdfind"
        );
        let caution = risk::assess("rm -rf build");
        let m = final_message("⚡".into(), &[], &[], &caution);
        assert!(m.starts_with("⚡ · ⚠ caution · deletes the folder"), "{m}");
        let destructive = risk::assess("mkfs.ext4 /dev/sdb1");
        let m = final_message("⚡".into(), &[], &[], &destructive);
        assert_eq!(
            m,
            "⚡ · ⚠ destructive · review before running — it never runs by itself"
        );
    }

    #[test]
    fn a_device_is_described_by_what_lsblk_knows() {
        i18n::set(i18n::Lang::En);
        let known = devices::parse_lsblk(
            r#"{"blockdevices":[
              {"name":"sda","model":"Samsung SSD","size":"500G","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/sda"},
              {"name":"sdb","model":"SanDisk Ultra","size":"29.8G","fstype":"vfat","mountpoint":"/media/pk/USB","rm":true,"path":"/dev/sdb"}]}"#,
        )
        .unwrap();
        assert_eq!(
            describe_device(&known, "/dev/sdb"),
            "/dev/sdb  SanDisk Ultra  29.8G  vfat · mounted at /media/pk/USB"
        );
        let system = describe_device(&known, "/dev/sda");
        assert!(system.contains("NOT removable"), "{system}");
        assert!(describe_device(&known, "/dev/sdz").contains("not found on this machine"));
        // a placeholder that was never filled in is not a device on this machine either
        assert!(describe_device(&known, "{{/dev/sdX}}").contains("not found"));
    }
}
