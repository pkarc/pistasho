//! Candidate list and placeholder fill-in. Draws on `/dev/tty` so stdout stays free
//! and the zsh widget can capture the chosen command.

use crate::config::Height;
use crate::i18n::tr;
use crate::index::{Entry, Source};
use crate::placeholders::{self, Part};
use crate::search::Searcher;
use anyhow::{Context, Result};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{position, MoveTo};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{Frame, Terminal, TerminalOptions, Viewport};
use std::fs::File;
use std::io::Write;
use std::os::fd::AsRawFd;

const MAX_RESULTS: usize = 300;
/// Rows of the fill-only viewport: input box (3) + command preview (3) + hint (1).
const FILL_ONLY_ROWS: u16 = 7;
/// Rows of the single-box list (explore / alternatives): 2 borders, the filter line, 8 rows.
const COMPACT_ROWS: u16 = 11;

// Palette (Tokyo Night)
const FG: Color = Color::Rgb(0xc0, 0xca, 0xf5);
const GRAY: Color = Color::Rgb(0x56, 0x5f, 0x89);
const MAG: Color = Color::Rgb(0xbb, 0x9a, 0xf7);
const CYAN: Color = Color::Rgb(0x7d, 0xcf, 0xff);
const GREEN: Color = Color::Rgb(0x9e, 0xce, 0x6a);
const YEL: Color = Color::Rgb(0xe0, 0xaf, 0x68);
const ORANGE: Color = Color::Rgb(0xff, 0x9e, 0x64);
const SEL: Color = Color::Rgb(0x28, 0x34, 0x57);

pub enum Outcome {
    /// The command, and where it came from (`tldr · common/fc`, `cheats.toml`, `history`);
    /// the origin is empty when it does not apply.
    Selected(String, String),
    Cancelled,
}

/// What the list is for (ADR 0001).
pub enum ListMode {
    /// `pistasho search` and the tmux popup: the classic four boxes with details.
    Search,
    /// Everything — your cheats, history and tldr — in one box with a filter.
    Explore,
    /// The second `Ctrl-Space`: the candidates for `question`, which stay on screen while the
    /// filter is empty; typing searches everything. `candidates` and `current` are indices into
    /// the entries given to [`run`]; `current` is the command on the prompt, selected at first.
    /// With `send` the hint says the pick is sent to a tmux pane rather than replacing a command
    /// (the popup has no prompt to replace: this list is where its answer is shown).
    Alternatives {
        question: String,
        candidates: Vec<usize>,
        current: Option<usize>,
        send: bool,
    },
}

pub struct Options {
    pub initial_query: String,
    pub fill: bool,
    /// Full screen (alternate screen) or compact below the prompt.
    pub height: Height,
    /// Skip the search and go straight to filling the placeholders of this command.
    pub start_fill: Option<String>,
    pub mode: ListMode,
}

struct Fill {
    template: String,
    names: Vec<String>,
    values: Vec<Option<String>>,
    cur: usize,
    input: String,
}

impl Fill {
    fn new(template: &str) -> Self {
        let names = placeholders::names(template);
        Self {
            template: template.to_string(),
            values: vec![None; names.len()],
            names,
            cur: 0,
            input: String::new(),
        }
    }
}

struct App<'a> {
    entries: &'a [Entry],
    searcher: Searcher,
    query: String,
    results: Vec<usize>,
    sel: usize,
    offset: usize,
    fill: Option<Fill>,
    fill_enabled: bool,
    /// Only the placeholder fill-in, no search: a short viewport without list or details.
    fill_only: bool,
    mode: ListMode,
    /// Origin of the entry whose placeholders are being filled.
    fill_origin: String,
}

impl<'a> App<'a> {
    fn refresh(&mut self) {
        if self.query.trim().is_empty() {
            if let ListMode::Alternatives {
                candidates, current, ..
            } = &self.mode
            {
                self.results = candidates.clone();
                self.sel = current
                    .and_then(|c| candidates.iter().position(|&i| i == c))
                    .unwrap_or(0);
                self.offset = 0;
                return;
            }
        }
        self.results = self.searcher.rank(self.entries, &self.query, MAX_RESULTS);
        self.sel = 0;
        self.offset = 0;
    }

    fn selected(&self) -> Option<&'a Entry> {
        self.results.get(self.sel).map(|&i| &self.entries[i])
    }
}

/// Restores the terminal even on a panic.
struct Guard {
    alternate_screen: bool,
    /// Original stdout, saved while it points at the tty (see `redirect_stdout_to`).
    saved_stdout: Option<i32>,
}

/// Points stdout at the tty and returns the saved original. ratatui and crossterm query the
/// cursor position through stdout, which inside the zsh widget is the pipe of `$(...)`, so the
/// terminal's answer would never come back. The chosen command is printed after `run` returns.
fn redirect_stdout_to(tty: &File) -> Option<i32> {
    // SAFETY: plain fd duplication on descriptors we own; failures are checked.
    unsafe {
        let saved = libc::dup(libc::STDOUT_FILENO);
        if saved < 0 {
            return None;
        }
        if libc::dup2(tty.as_raw_fd(), libc::STDOUT_FILENO) < 0 {
            libc::close(saved);
            return None;
        }
        Some(saved)
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(saved) = self.saved_stdout.take() {
            // SAFETY: `saved` is the duplicate made in `redirect_stdout_to`.
            unsafe {
                libc::dup2(saved, libc::STDOUT_FILENO);
                libc::close(saved);
            }
        }
        let _ = disable_raw_mode();
        if self.alternate_screen {
            if let Ok(mut tty) = File::options().write(true).open("/dev/tty") {
                let _ = execute!(tty, LeaveAlternateScreen);
            }
        }
    }
}

/// The terminal while an interface is up: raw mode, and either the alternate screen or a
/// viewport of a few lines below the prompt. `close` erases the viewport and gives the
/// terminal back.
struct Screen {
    terminal: Terminal<CrosstermBackend<File>>,
    tty_w: File,
    /// Column of the prompt cursor, to put it back where zle expects it when we are done.
    origin_col: u16,
    inline: bool,
    _guard: Guard,
}

impl Screen {
    /// `Some(rows)` for a viewport of that many lines below the prompt, `None` for the
    /// alternate screen.
    fn open(inline_rows: Option<u16>) -> Result<Screen> {
        let tty = File::options()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .context(tr("could not open /dev/tty", "no pude abrir /dev/tty"))?;
        let mut tty_w = tty.try_clone()?;
        enable_raw_mode()?;
        let guard = Guard {
            alternate_screen: inline_rows.is_none(),
            saved_stdout: if inline_rows.is_some() {
                redirect_stdout_to(&tty_w)
            } else {
                None
            },
        };
        let mut origin_col = 0;
        let terminal = match inline_rows {
            None => {
                execute!(tty_w, EnterAlternateScreen)?;
                Terminal::new(CrosstermBackend::new(tty))?
            }
            Some(rows) => {
                origin_col = position().map(|(col, _)| col).unwrap_or(0);
                // start on the line below the prompt so the prompt itself stays visible
                tty_w.write_all(b"\r\n")?;
                tty_w.flush()?;
                Terminal::with_options(
                    CrosstermBackend::new(tty),
                    TerminalOptions {
                        viewport: Viewport::Inline(rows),
                    },
                )?
            }
        };
        Ok(Screen {
            terminal,
            tty_w,
            origin_col,
            inline: inline_rows.is_some(),
            _guard: guard,
        })
    }

    fn close(mut self) -> Result<()> {
        if self.inline {
            // erase the viewport and return to the prompt line (just above it)
            let prompt_row = self.terminal.get_frame().area().y.saturating_sub(1);
            self.terminal.clear()?;
            execute!(self.tty_w, MoveTo(self.origin_col, prompt_row))?;
        }
        self.terminal.show_cursor()?;
        Ok(())
    }
}

fn terminal_rows() -> u16 {
    size().map(|(_, rows)| rows).unwrap_or(24)
}

pub fn run(entries: &[Entry], opts: Options) -> Result<Outcome> {
    let fill_only = opts
        .start_fill
        .as_deref()
        .is_some_and(placeholders::has_placeholders);
    let term_rows = terminal_rows();
    let compact = !matches!(opts.mode, ListMode::Search);
    let inline_rows = if fill_only {
        // input box + command preview + hint line, whatever `ui.height` says
        Some(FILL_ONLY_ROWS.min(term_rows.saturating_sub(1).max(1)))
    } else if compact {
        Some(COMPACT_ROWS.min(term_rows.saturating_sub(1).max(1)))
    } else {
        opts.height.rows(term_rows)
    };
    let mut screen = Screen::open(inline_rows)?;

    let mut app = App {
        entries,
        searcher: Searcher::new(entries),
        query: opts.initial_query,
        results: Vec::new(),
        sel: 0,
        offset: 0,
        fill: opts
            .start_fill
            .as_deref()
            .filter(|t| placeholders::has_placeholders(t))
            .map(Fill::new),
        fill_enabled: opts.fill,
        fill_only,
        mode: opts.mode,
        fill_origin: String::new(),
    };
    app.refresh();

    loop {
        screen.terminal.draw(|f| draw(f, &mut app))?;
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let done = if app.fill.is_some() {
            on_fill_key(&mut app, key)
        } else {
            on_search_key(&mut app, key)
        };
        if let Some(outcome) = done {
            screen.close()?;
            return Ok(outcome);
        }
    }
}

// ---------------------------------------------------------------- install confirmation

/// What the confirmation box shows (ADR 0001, case 4 of the cases GIF).
pub struct InstallBox {
    /// The command the answer will put on the prompt.
    pub command: String,
    /// `Ubuntu 24.04.5 LTS · apt`
    pub system: String,
    /// `sudo apt install -y tree`: exactly what will run if the user says yes.
    pub install: String,
    /// The programs that are not installed.
    pub missing: Vec<String>,
    /// Whether `[a]`, an alternative that needs nothing new, is on offer.
    pub alternative: bool,
}

pub enum InstallChoice {
    Install,
    Alternative,
    Cancel,
}

/// Rows of the box: 2 borders, the warning, three fields, a blank line and the keys.
const INSTALL_BOX_ROWS: u16 = 8;

/// The key a user presses in the install box. `Enter` is deliberately not one of them: running
/// a package manager needs an explicit `y` (or `s`, in Spanish), never a habit.
fn install_key(key: KeyEvent, alternative: bool) -> Option<InstallChoice> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('y' | 's') if !ctrl => Some(InstallChoice::Install),
        KeyCode::Char('a') if !ctrl && alternative => Some(InstallChoice::Alternative),
        KeyCode::Esc | KeyCode::Char('n') if !ctrl => Some(InstallChoice::Cancel),
        KeyCode::Char('c' | 'g') if ctrl => Some(InstallChoice::Cancel),
        _ => None,
    }
}

/// Shows what the answer needs and what would be installed, and waits for the user's choice.
/// Nothing is run here.
pub fn confirm_install(info: &InstallBox) -> Result<InstallChoice> {
    let rows = INSTALL_BOX_ROWS.min(terminal_rows().saturating_sub(1).max(1));
    let mut screen = Screen::open(Some(rows))?;
    let choice = loop {
        screen.terminal.draw(|f| draw_install(f, info))?;
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(choice) = install_key(key, info.alternative) {
            break choice;
        }
    };
    screen.close()?;
    Ok(choice)
}

fn draw_install(f: &mut Frame, info: &InstallBox) {
    let b = Block::bordered()
        .border_style(Style::default().fg(YEL))
        .title(Span::styled(
            format!(" {} ", tr("suggested installation", "instalación sugerida")),
            Style::default().fg(YEL).add_modifier(Modifier::BOLD),
        ));
    let inner = b.inner(f.area());
    f.render_widget(b, f.area());

    let field = |label: &str, value: &str, color: Color| {
        Line::from(vec![
            Span::styled(format!(" {label:<10}"), Style::default().fg(GRAY)),
            Span::styled(
                value.to_string(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ])
    };
    let key = |k: &str, text: &str, color: Color| {
        vec![
            Span::styled(
                format!("[{k}]"),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {text}   "), Style::default().fg(FG)),
        ]
    };
    let mut keys = vec![Span::raw(" ")];
    keys.extend(key(tr("y", "s"), tr("install", "instalar"), GREEN));
    if info.alternative {
        keys.extend(key(
            "a",
            tr("alternative without installing", "alternativa sin instalar"),
            YEL,
        ));
    }
    keys.extend(key("esc", tr("cancel", "cancelar"), GRAY));

    let lines = vec![
        Line::styled(
            format!(
                " ⚠ {} {}",
                info.missing.join(", "),
                tr("is not installed", "no está instalado")
            ),
            Style::default().fg(YEL).add_modifier(Modifier::BOLD),
        ),
        field(tr("Command:", "Comando:"), &info.command, FG),
        field(tr("System:", "Sistema:"), &info.system, FG),
        field(tr("Install:", "Instalar:"), &info.install, CYAN),
        Line::raw(""),
        Line::from(keys),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(c)
}

fn on_search_key(app: &mut App, key: KeyEvent) -> Option<Outcome> {
    if key.code == KeyCode::Esc || ctrl(&key, 'c') || ctrl(&key, 'g') {
        return Some(Outcome::Cancelled);
    }
    if key.code == KeyCode::Up || ctrl(&key, 'p') {
        app.sel = app.sel.saturating_sub(1);
        return None;
    }
    if key.code == KeyCode::Down || ctrl(&key, 'n') {
        if app.sel + 1 < app.results.len() {
            app.sel += 1;
        }
        return None;
    }
    if ctrl(&key, 'u') {
        app.query.clear();
        app.refresh();
        return None;
    }
    if ctrl(&key, 'w') {
        let trimmed = app.query.trim_end().len();
        app.query.truncate(trimmed);
        let cut = app.query.rfind(' ').map(|i| i + 1).unwrap_or(0);
        app.query.truncate(cut);
        app.refresh();
        return None;
    }
    match key.code {
        KeyCode::Enter => {
            let entry = app.selected()?;
            if app.fill_enabled && placeholders::has_placeholders(&entry.cmd) {
                app.fill = Some(Fill::new(&entry.cmd));
                app.fill_origin = entry.origin.clone();
                None
            } else {
                Some(Outcome::Selected(entry.cmd.clone(), entry.origin.clone()))
            }
        }
        // Tab: insert the template as is, with the {{placeholders}}
        KeyCode::Tab => app
            .selected()
            .map(|e| Outcome::Selected(e.cmd.clone(), e.origin.clone())),
        KeyCode::Backspace => {
            app.query.pop();
            app.refresh();
            None
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.query.push(c);
            app.refresh();
            None
        }
        _ => None,
    }
}

fn on_fill_key(app: &mut App, key: KeyEvent) -> Option<Outcome> {
    let origin = app.fill_origin.clone();
    let fill = app.fill.as_mut()?;
    // Esc: insert with the placeholders still missing
    if key.code == KeyCode::Esc || ctrl(&key, 'g') {
        return Some(Outcome::Selected(
            placeholders::fill(&fill.template, &fill.values),
            origin,
        ));
    }
    if ctrl(&key, 'c') {
        return Some(Outcome::Cancelled);
    }
    match key.code {
        KeyCode::Enter | KeyCode::Tab => {
            let v = std::mem::take(&mut fill.input);
            fill.values[fill.cur] = if key.code == KeyCode::Tab || v.is_empty() {
                None
            } else {
                Some(v)
            };
            fill.cur += 1;
            if fill.cur >= fill.names.len() {
                return Some(Outcome::Selected(
                    placeholders::fill(&fill.template, &fill.values),
                    origin,
                ));
            }
            if let Some(prev) = &fill.values[fill.cur] {
                fill.input = prev.clone();
            }
        }
        KeyCode::BackTab if fill.cur > 0 => {
            fill.cur -= 1;
            fill.input = fill.values[fill.cur].clone().unwrap_or_default();
        }
        KeyCode::Backspace => {
            fill.input.pop();
        }
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => fill.input.push(c),
        _ => {}
    }
    None
}

// ---------------------------------------------------------------- drawing

fn tag_color(s: Source) -> Color {
    match s {
        Source::Tldr => CYAN,
        Source::Cheat => GREEN,
        Source::Hist => YEL,
    }
}

/// Spans of a command with the {{placeholders}} highlighted.
fn cmd_spans(cmd: &str, bg: Option<Color>) -> Vec<Span<'static>> {
    let base = |c: Color| match bg {
        Some(b) => Style::default().fg(c).bg(b),
        None => Style::default().fg(c),
    };
    // multiline history commands are shown on a single line
    let flat = cmd.replace('\n', " ⏎ ");
    placeholders::parse(&flat)
        .into_iter()
        .map(|p| match p {
            Part::Text(t) => Span::styled(t, base(FG)),
            Part::Placeholder(n) => Span::styled(format!("{{{{{n}}}}}"), base(ORANGE)),
        })
        .collect()
}

fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let compact = !matches!(app.mode, ListMode::Search);
    if app.fill_only || (compact && app.fill.is_some()) {
        let [top, preview, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(area);
        draw_fill(f, app, top);
        draw_list(f, app, preview);
        f.render_widget(
            Paragraph::new(Line::styled(footer_hint(true), Style::default().fg(GRAY))),
            footer,
        );
        return;
    }
    if compact {
        draw_compact(f, app, area);
        return;
    }
    let [top, list, detail, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(4),
        Constraint::Length(1),
    ])
    .areas(area);

    if app.fill.is_some() {
        draw_fill(f, app, top);
    } else {
        draw_query(f, app, top);
    }
    draw_list(f, app, list);
    draw_detail(f, app, detail);

    f.render_widget(
        Paragraph::new(Line::styled(
            footer_hint(app.fill.is_some()),
            Style::default().fg(GRAY),
        )),
        footer,
    );
}

/// Key hints for the last line, for the placeholder fill-in or for the list.
fn footer_hint(filling: bool) -> &'static str {
    if filling {
        tr(
            " ⏎ next · tab skip · shift-tab back · esc insert as is · ctrl-c cancel",
            " ⏎ siguiente · tab saltar · shift-tab volver · esc insertar así · ctrl-c cancelar",
        )
    } else {
        tr(
            " ↑↓ move · ⏎ insert · tab without filling · ctrl-u clear · esc quit",
            " ↑↓ mover · ⏎ insertar · tab sin rellenar · ctrl-u borrar · esc salir",
        )
    }
}

fn block(title: &str) -> Block<'static> {
    Block::bordered()
        .border_style(Style::default().fg(GRAY))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(MAG).add_modifier(Modifier::BOLD),
        ))
}

fn draw_query(f: &mut Frame, app: &App, area: Rect) {
    let count = format!("{}/{}", app.results.len(), app.entries.len());
    let line = Line::from(vec![
        Span::styled("❯ ", Style::default().fg(MAG).add_modifier(Modifier::BOLD)),
        Span::styled(
            app.query.clone(),
            Style::default().fg(FG).add_modifier(Modifier::BOLD),
        ),
    ]);
    let b = block("pistasho")
        .title_bottom(Line::styled(format!(" {count} "), Style::default().fg(GRAY)).right_aligned());
    let inner = b.inner(area);
    f.render_widget(Paragraph::new(line).block(b), area);
    let x = inner.x + 2 + app.query.chars().count() as u16;
    f.set_cursor_position((x.min(inner.right().saturating_sub(1)), inner.y));
}

fn draw_fill(f: &mut Frame, app: &App, area: Rect) {
    let fill = app.fill.as_ref().unwrap();
    let name = placeholders::display_name(&fill.names[fill.cur]).to_string();
    let label = format!("{name} › ");
    let line = Line::from(vec![
        Span::styled(
            label.clone(),
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ),
        Span::styled(fill.input.clone(), Style::default().fg(FG)),
    ]);
    let title = format!("{} {}/{}", tr("fill", "rellenar"), fill.cur + 1, fill.names.len());
    let b = block(&title);
    let inner = b.inner(area);
    f.render_widget(Paragraph::new(line).block(b), area);
    let x = inner.x + (label.chars().count() + fill.input.chars().count()) as u16;
    f.set_cursor_position((x.min(inner.right().saturating_sub(1)), inner.y));
}

fn draw_list(f: &mut Frame, app: &mut App, area: Rect) {
    let b = Block::bordered().border_style(Style::default().fg(GRAY));
    let inner = b.inner(area);
    f.render_widget(b, area);
    let h = inner.height as usize;
    if h == 0 {
        return;
    }

    if let Some(fill) = &app.fill {
        // command preview with the current placeholder highlighted
        let mut spans = vec![Span::raw(" ")];
        let mut i = 0;
        for p in placeholders::parse(&fill.template) {
            match p {
                Part::Text(t) => spans.push(Span::styled(t, Style::default().fg(FG))),
                Part::Placeholder(n) => {
                    let (text, style) = match (&fill.values.get(i).cloned().flatten(), i == fill.cur) {
                        (_, true) if !fill.input.is_empty() => {
                            (fill.input.clone(), Style::default().fg(GREEN).bg(SEL))
                        }
                        (_, true) => (format!("{{{{{n}}}}}"), Style::default().fg(ORANGE).bg(SEL)),
                        (Some(v), false) => (v.clone(), Style::default().fg(GREEN)),
                        (None, false) => (format!("{{{{{n}}}}}"), Style::default().fg(ORANGE)),
                    };
                    spans.push(Span::styled(text, style));
                    i += 1;
                }
            }
        }
        f.render_widget(Paragraph::new(Line::from(spans)), inner);
        return;
    }

    f.render_widget(Paragraph::new(list_lines(app, h)), inner);
}

/// The visible rows of the result list (scrolled so the selection shows), or a message when
/// there is nothing to list.
fn list_lines(app: &mut App, h: usize) -> Vec<Line<'static>> {
    if app.results.is_empty() {
        let msg = if app.entries.is_empty() {
            tr(
                " Empty index: run `pistasho update`",
                " Índice vacío: ejecuta `pistasho update`",
            )
        } else {
            tr(" no results", " sin resultados")
        };
        return vec![Line::styled(msg, Style::default().fg(GRAY))];
    }

    // scroll so the selection is always visible
    if app.sel < app.offset {
        app.offset = app.sel;
    } else if app.sel >= app.offset + h {
        app.offset = app.sel + 1 - h;
    }

    app.results
        .iter()
        .enumerate()
        .skip(app.offset)
        .take(h)
        .map(|(pos, &i)| {
            let e = &app.entries[i];
            let selected = pos == app.sel;
            let bg = selected.then_some(SEL);
            let st = |c: Color| match bg {
                Some(b) => Style::default().fg(c).bg(b),
                None => Style::default().fg(c),
            };
            let mut spans = vec![
                Span::styled(if selected { "▌" } else { " " }, st(MAG)),
                Span::styled(format!(" {:<5}", e.source.tag()), st(tag_color(e.source))),
            ];
            spans.extend(cmd_spans(&e.cmd, bg));
            let mut line = Line::from(spans);
            if selected {
                line = line.style(Style::default().bg(SEL));
            }
            line
        })
        .collect()
}

/// The single box of explore / alternatives: title, a `❯` filter line, the rows, and the key
/// hints on the bottom border (ADR 0001).
fn draw_compact(f: &mut Frame, app: &mut App, area: Rect) {
    let (title, hint) = match &app.mode {
        ListMode::Alternatives { question, send, .. } => (
            format!(
                "{} · «{}»",
                tr("other options", "otras opciones"),
                clip(question, (area.width as usize).saturating_sub(24))
            ),
            if *send {
                tr(
                    " ↑↓ see more · ⏎ send · esc cancel ",
                    " ↑↓ ver más · ⏎ enviar · esc cancelar ",
                )
            } else {
                tr(
                    " ↑↓ move · ⏎ replace · esc keep current ",
                    " ↑↓ mover · ⏎ reemplazar · esc dejar el actual ",
                )
            },
        ),
        _ => (
            tr(
                "explore · your cheats, history and tldr",
                "explorar · tus atajos, historial y tldr",
            )
            .to_string(),
            tr(
                " ↑↓ move · ⏎ pick · tab without filling · esc exit ",
                " ↑↓ mover · ⏎ elegir · tab sin rellenar · esc salir ",
            ),
        ),
    };
    let b = block(&title).title_bottom(Line::styled(hint, Style::default().fg(GRAY)).right_aligned());
    let inner = b.inner(area);
    f.render_widget(b, area);
    if inner.height == 0 {
        return;
    }
    let [filter, rows] = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    let line = Line::from(vec![
        Span::styled("❯ ", Style::default().fg(MAG).add_modifier(Modifier::BOLD)),
        Span::styled(
            app.query.clone(),
            Style::default().fg(FG).add_modifier(Modifier::BOLD),
        ),
    ]);
    f.render_widget(Paragraph::new(line), filter);
    let x = filter.x + 2 + app.query.chars().count() as u16;
    f.set_cursor_position((x.min(filter.right().saturating_sub(1)), filter.y));
    let lines = list_lines(app, rows.height as usize);
    f.render_widget(Paragraph::new(lines), rows);
}

/// `s` cut to at most `max` characters, with an ellipsis when it was longer.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let kept: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{kept}…")
    }
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    let b = Block::bordered().border_style(Style::default().fg(GRAY));
    let lines = match app.selected() {
        Some(e) if app.fill.is_none() => vec![
            Line::styled(format!(" {}", e.desc), Style::default().fg(FG)),
            Line::styled(format!(" {}", e.origin), Style::default().fg(GRAY)),
        ],
        _ => vec![],
    };
    f.render_widget(Paragraph::new(lines).block(b), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(tool: &str, cmd: &str, source: Source) -> Entry {
        Entry {
            cmd: cmd.into(),
            desc: String::new(),
            alt: String::new(),
            tool: tool.into(),
            source,
            origin: format!("origin of {cmd}"),
            count: 0,
        }
    }

    fn entries() -> Vec<Entry> {
        vec![
            entry("tar", "tar xf {{file}}", Source::Tldr),
            entry("ss", "ss -tlnp", Source::Tldr),
            entry("fuser", "fuser -k {{port}}/tcp", Source::Cheat),
            entry("ssh", "ssh root@host", Source::Hist),
            entry("uptime", "uptime --pretty", Source::Tldr),
        ]
    }

    fn app<'a>(entries: &'a [Entry], mode: ListMode) -> App<'a> {
        let mut app = App {
            entries,
            searcher: Searcher::new(entries),
            query: String::new(),
            results: Vec::new(),
            sel: 0,
            offset: 0,
            fill: None,
            fill_enabled: true,
            fill_only: false,
            mode,
            fill_origin: String::new(),
        };
        app.refresh();
        app
    }

    fn alternatives(candidates: Vec<usize>, current: Option<usize>) -> ListMode {
        ListMode::Alternatives {
            question: "how long has it been up".into(),
            candidates,
            current,
            send: false,
        }
    }

    #[test]
    fn alternatives_open_on_the_candidates_with_the_current_one_selected() {
        let e = entries();
        let a = app(&e, alternatives(vec![4, 1, 0], Some(1)));
        assert_eq!(
            a.results,
            vec![4, 1, 0],
            "the question's candidates, in their order"
        );
        assert_eq!(a.sel, 1, "the command on the prompt is the selected row");
        assert_eq!(a.selected().unwrap().tool, "ss");
    }

    #[test]
    fn alternatives_without_a_known_current_start_at_the_top() {
        let e = entries();
        let a = app(&e, alternatives(vec![4, 1], None));
        assert_eq!((a.results.clone(), a.sel), (vec![4, 1], 0));
    }

    #[test]
    fn typing_searches_everything_and_clearing_returns_to_the_candidates() {
        let e = entries();
        let mut a = app(&e, alternatives(vec![4], Some(4)));
        a.query = "fuser".into();
        a.refresh();
        assert_eq!(e[a.results[0]].tool, "fuser", "not limited to the candidates");
        a.query.clear();
        a.refresh();
        assert_eq!(a.results, vec![4]);
        assert_eq!(a.sel, 0);
    }

    #[test]
    fn explore_lists_cheats_then_history_then_tldr() {
        let e = entries();
        let a = app(&e, ListMode::Explore);
        let sources: Vec<Source> = a.results.iter().map(|&i| e[i].source).collect();
        assert_eq!(
            sources,
            vec![
                Source::Cheat,
                Source::Hist,
                Source::Tldr,
                Source::Tldr,
                Source::Tldr
            ]
        );
    }

    #[test]
    fn enter_on_a_row_without_placeholders_returns_it_with_its_origin() {
        let e = entries();
        let mut a = app(&e, alternatives(vec![1, 0], Some(1)));
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        match on_search_key(&mut a, key) {
            Some(Outcome::Selected(cmd, origin)) => {
                assert_eq!(cmd, "ss -tlnp");
                assert_eq!(origin, "origin of ss -tlnp");
            }
            _ => panic!("expected a selection"),
        }
    }

    #[test]
    fn enter_on_a_row_with_placeholders_asks_for_them_and_remembers_the_origin() {
        let e = entries();
        let mut a = app(&e, alternatives(vec![0], Some(0)));
        assert!(on_search_key(&mut a, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).is_none());
        assert!(a.fill.is_some());
        let done = on_fill_key(&mut a, KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .or_else(|| on_fill_key(&mut a, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        match done {
            Some(Outcome::Selected(cmd, origin)) => {
                assert_eq!(cmd, "tar xf x");
                assert_eq!(origin, "origin of tar xf {{file}}");
            }
            _ => panic!("expected the filled command"),
        }
    }

    #[test]
    fn escape_keeps_the_current_command() {
        let e = entries();
        let mut a = app(&e, alternatives(vec![1], Some(1)));
        let out = on_search_key(&mut a, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(matches!(out, Some(Outcome::Cancelled)));
    }

    fn press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn choice(key: KeyEvent, alternative: bool) -> &'static str {
        match install_key(key, alternative) {
            Some(InstallChoice::Install) => "install",
            Some(InstallChoice::Alternative) => "alternative",
            Some(InstallChoice::Cancel) => "cancel",
            None => "nothing",
        }
    }

    /// Installing runs a package manager as root: only an explicit `y` / `s` does it.
    #[test]
    fn only_an_explicit_key_installs() {
        let none = KeyModifiers::NONE;
        assert_eq!(choice(press(KeyCode::Char('y'), none), false), "install");
        assert_eq!(choice(press(KeyCode::Char('s'), none), false), "install");
        // Enter, Space, Tab and anything else do nothing: the box stays up
        for code in [
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Tab,
            KeyCode::Char('x'),
            KeyCode::Down,
        ] {
            assert_eq!(choice(press(code, none), true), "nothing", "{code:?}");
        }
        // Ctrl-Y is not Y
        assert_eq!(
            choice(press(KeyCode::Char('y'), KeyModifiers::CONTROL), false),
            "nothing"
        );
    }

    #[test]
    fn the_box_can_be_cancelled_and_offers_the_alternative_only_when_there_is_one() {
        let none = KeyModifiers::NONE;
        assert_eq!(choice(press(KeyCode::Esc, none), false), "cancel");
        assert_eq!(choice(press(KeyCode::Char('n'), none), false), "cancel");
        assert_eq!(
            choice(press(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            "cancel"
        );
        assert_eq!(choice(press(KeyCode::Char('a'), none), true), "alternative");
        assert_eq!(
            choice(press(KeyCode::Char('a'), none), false),
            "nothing",
            "no alternative to pick"
        );
    }

    #[test]
    fn long_titles_are_clipped_with_an_ellipsis() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
        assert_eq!(clip("anything", 0), "…");
    }
}
