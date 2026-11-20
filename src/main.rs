mod config;
mod eval;
mod history;
mod index;
mod placeholders;
mod search;
mod shell;
mod tldr;
mod tui;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use config::Config;
use index::{Cheat, Index};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "pistasho",
    version,
    about = "La pista del comando que necesitas, sin salir de la terminal"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Abre el buscador e imprime el comando elegido
    Search {
        /// Texto inicial de búsqueda
        #[arg(short, long, default_value = "")]
        query: String,
        /// Enviar el comando a un panel de tmux en vez de imprimirlo
        #[arg(long, value_name = "PANE")]
        send_to: Option<String>,
        /// No pedir valores para los {{marcadores}}
        #[arg(long)]
        no_fill: bool,
    },
    /// Búsqueda sin interfaz: imprime los mejores resultados
    Query {
        query: Vec<String>,
        #[arg(short = 'n', long, default_value_t = 10)]
        limit: usize,
    },
    /// Descarga las páginas de tldr y reconstruye el índice
    Update,
    /// Agrega un comando a tu cheatsheet
    Add {
        /// El comando; usa {{nombre}} para los marcadores
        cmd: String,
        #[arg(short, long)]
        desc: String,
        #[arg(short, long, value_delimiter = ',')]
        tags: Vec<String>,
    },
    /// Imprime el código de integración para tu shell o tmux
    Init {
        #[arg(value_enum)]
        target: InitTarget,
    },
    /// Asistente de configuración
    Config,
    /// Mide la precisión del buscador con un conjunto de preguntas
    Eval {
        #[arg(default_value = "eval/preguntas.toml")]
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
    match cli.command {
        Command::Search {
            query,
            send_to,
            no_fill,
        } => {
            let cfg = Config::load()?;
            let idx = Index::load(&cfg)?;
            if !idx.has_tldr && idx.entries.is_empty() {
                eprintln!("pistasho: aún no hay índice; ejecuta `pistasho config` o `pistasho update`");
                return Ok(ExitCode::from(2));
            }
            let outcome = tui::run(
                &idx.entries,
                tui::Options {
                    initial_query: query,
                    fill: !no_fill,
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
        Command::Query { query, limit } => {
            let cfg = Config::load()?;
            let idx = Index::load(&cfg)?;
            let mut s = search::Searcher::new(&idx.entries);
            for i in s.rank(&idx.entries, &query.join(" "), limit) {
                let e = &idx.entries[i];
                println!("{:<5} {}\n      {}", e.source.tag(), e.cmd, e.desc);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Update => {
            let cfg = Config::load()?;
            let (pages, n) = index::update(&cfg)?;
            eprintln!("✓ {pages} páginas, {n} ejemplos ({})", cfg.languages.join(", "));
            Ok(ExitCode::SUCCESS)
        }
        Command::Add { cmd, desc, tags } => {
            let path = index::add_cheat(Cheat { cmd, desc, tags })?;
            eprintln!("✓ Agregado a {}", path.display());
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
                eprintln!("✓ {pages} páginas, {n} ejemplos");
            }
            println!("\nSiguiente paso, en ~/.zshrc:\n  eval \"$(pistasho init zsh)\"");
            println!("Y para tmux:\n  pistasho init tmux >> ~/.tmux.conf");
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

fn send_to_tmux(pane: &str, cmd: &str) -> Result<()> {
    let status = std::process::Command::new("tmux")
        .args(["send-keys", "-t", pane, "-l", "--", cmd])
        .status()
        .context("no pude ejecutar tmux")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys falló ({status})");
    }
    Ok(())
}
