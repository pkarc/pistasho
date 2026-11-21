//! Candidate list and placeholder fill-in. Draws on `/dev/tty` so stdout stays free
//! and the zsh widget can capture the chosen command.

use crate::config::Height;
use crate::devices;
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
const RED: Color = Color::Rgb(0xf7, 0x76, 0x8e);

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

/// Removable devices offered for a placeholder that stands for one, and the disks left out.
/// Only these can be picked: a typo or a hasty Enter must not point `mkfs` at the system disk.
struct Devices {
    offered: Vec<devices::Device>,
    /// Names of the disks that are not offered, for the note under the list.
    hidden: Vec<String>,
    sel: usize,
}

impl Devices {
    fn from_list(all: &[devices::Device]) -> Self {
        Self {
            offered: devices::removable(all).into_iter().cloned().collect(),
            hidden: devices::hidden_disks(all)
                .into_iter()
                .map(str::to_string)
                .collect(),
            sel: 0,
        }
    }
}

/// Rows the device list can show at once (the box adds the borders and the note).
const DEVICE_ROWS: usize = 4;

struct Fill {
    template: String,
    names: Vec<String>,
    values: Vec<Option<String>>,
    cur: usize,
    input: String,
    /// Which placeholders stand for a device.
    is_device: Vec<bool>,
    devices: Option<Devices>,
}

impl Fill {
    /// Asks `lsblk` only if the template has a device placeholder.
    fn new(template: &str) -> Self {
        let wants = placeholders::names(template)
            .iter()
            .any(|n| devices::placeholder_names_a_device(n));
        let list = if wants { devices::lsblk().ok() } else { None };
        Self::with_devices(template, list.as_deref())
    }

    fn with_devices(template: &str, all: Option<&[devices::Device]>) -> Self {
        let names = placeholders::names(template);
        Self {
            template: template.to_string(),
            values: vec![None; names.len()],
            is_device: names
                .iter()
                .map(|n| devices::placeholder_names_a_device(n))
                .collect(),
            names,
            cur: 0,
            input: String::new(),
            devices: all.map(Devices::from_list),
        }
    }

    /// The current placeholder is a device and there are removable ones to choose from: the
    /// list takes the place of the text input.
    fn picking(&self) -> bool {
        self.is_device[self.cur]
            && self.values[self.cur].is_none()
            && self.devices.as_ref().is_some_and(|d| !d.offered.is_empty())
    }

    /// Moves to the next placeholder, or finishes with the filled-in command.
    fn advance(&mut self, origin: String) -> Option<Outcome> {
        self.cur += 1;
        if self.cur >= self.names.len() {
            return Some(Outcome::Selected(
                placeholders::fill(&self.template, &self.values),
                origin,
            ));
        }
        if let Some(prev) = &self.values[self.cur] {
            self.input = prev.clone();
        }
        None
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
    // a device placeholder swaps the one-line input for a list of up to DEVICE_ROWS devices
    let device_rows = opts.start_fill.as_deref().is_some_and(|t| {
        placeholders::names(t)
            .iter()
            .any(|n| devices::placeholder_names_a_device(n))
    });
    let inline_rows = if fill_only {
        // input box + command preview + hint line, whatever `ui.height` says
        let rows = FILL_ONLY_ROWS + if device_rows { DEVICE_ROWS as u16 } else { 0 };
        Some(rows.min(term_rows.saturating_sub(1).max(1)))
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
    /// `sudo apt install -y tree`: exactly what will run if the user says yes. `None` when no
    /// package provides the program, so there is nothing to install.
    pub install: Option<String>,
    /// The programs that are not installed.
    pub missing: Vec<String>,
    /// `[a]`: a command made of tools that are already installed, and what it is.
    pub alternative: Option<String>,
}

pub enum InstallChoice {
    Install,
    Alternative,
    Cancel,
}

/// Rows of the box: 2 borders, the warning, the command and system, a blank line, the keys; plus
/// a row for the install command and one for the alternative when there are such.
fn install_box_rows(info: &InstallBox) -> u16 {
    6 + 1 + u16::from(info.install.is_some()) + u16::from(info.alternative.is_some())
}

/// The key a user presses in the install box. `Enter` is deliberately not one of them: running
/// a package manager needs an explicit `y` (or `s`, in Spanish), never a habit.
///
/// `y` only counts when there is something to install, `a` only when there is an alternative.
fn install_key(key: KeyEvent, can_install: bool, has_alternative: bool) -> Option<InstallChoice> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('y' | 's') if !ctrl && can_install => Some(InstallChoice::Install),
        KeyCode::Char('a') if !ctrl && has_alternative => Some(InstallChoice::Alternative),
        KeyCode::Esc | KeyCode::Char('n') if !ctrl => Some(InstallChoice::Cancel),
        KeyCode::Char('c' | 'g') if ctrl => Some(InstallChoice::Cancel),
        _ => None,
    }
}

/// Shows what the answer needs and what would be installed, and waits for the user's choice.
/// Nothing is run here.
pub fn confirm_install(info: &InstallBox) -> Result<InstallChoice> {
    let rows = install_box_rows(info).min(terminal_rows().saturating_sub(1).max(1));
    let mut screen = Screen::open(Some(rows))?;
    let choice = loop {
        screen.terminal.draw(|f| draw_install(f, info))?;
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(choice) = install_key(key, info.install.is_some(), info.alternative.is_some()) {
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
    if info.install.is_some() {
        keys.extend(key(tr("y", "s"), tr("install", "instalar"), GREEN));
    }
    if info.alternative.is_some() {
        keys.extend(key(
            "a",
            tr("alternative without installing", "alternativa sin instalar"),
            YEL,
        ));
    }
    keys.extend(key("esc", tr("cancel", "cancelar"), GRAY));

    let mut lines = vec![
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
    ];
    if let Some(install) = &info.install {
        lines.push(field(tr("Install:", "Instalar:"), install, CYAN));
    }
    if let Some(alternative) = &info.alternative {
        lines.push(field(tr("Instead:", "En su lugar:"), alternative, YEL));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(keys));
    f.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------- destructive command box

/// What the red box shows (ADR 0001, case 3 of the cases GIF).
pub struct RiskBox {
    pub command: String,
    /// What the command does, in a sentence.
    pub effect: String,
    /// What the device is, when the command writes to one: `SanDisk Ultra  29.8G  vfat`.
    pub device: Option<String>,
    /// The fixed rules that matched (`mkfs`, `dd`…).
    pub signals: String,
    /// Whether `e` can be offered: a multiline command cannot be edited on one line.
    pub can_edit: bool,
}

pub enum RiskChoice {
    /// Put it on the prompt anyway. It is not run: the Enter that runs it is the user's own.
    Leave,
    Edit,
    Cancel,
}

fn risk_key(key: KeyEvent, can_edit: bool) -> Option<RiskChoice> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Enter => Some(RiskChoice::Leave),
        KeyCode::Char('e') if !ctrl && can_edit => Some(RiskChoice::Edit),
        KeyCode::Esc | KeyCode::Char('n') if !ctrl => Some(RiskChoice::Cancel),
        KeyCode::Char('c' | 'g') if ctrl => Some(RiskChoice::Cancel),
        _ => None,
    }
}

fn risk_box_rows(info: &RiskBox) -> u16 {
    // 2 borders, the title is on the border; command, effect, signals, a blank line and the keys
    2 + 3 + 1 + 1 + u16::from(info.device.is_some())
}

/// Shows why a command is dangerous and waits for the user to leave it, edit it or drop it.
/// Nothing is run here, and leaving it only types it on the prompt.
pub fn confirm_risk(info: &RiskBox) -> Result<RiskChoice> {
    let rows = risk_box_rows(info).min(terminal_rows().saturating_sub(1).max(1));
    let mut screen = Screen::open(Some(rows))?;
    let choice = loop {
        screen.terminal.draw(|f| draw_risk(f, info))?;
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if let Some(choice) = risk_key(key, info.can_edit) {
            break choice;
        }
    };
    screen.close()?;
    Ok(choice)
}

fn draw_risk(f: &mut Frame, info: &RiskBox) {
    let b = Block::bordered()
        .border_style(Style::default().fg(RED))
        .title(Span::styled(
            format!(" ⚠ {} ", tr("destructive command", "comando destructivo")),
            Style::default().fg(RED).add_modifier(Modifier::BOLD),
        ));
    let inner = b.inner(f.area());
    f.render_widget(b, f.area());

    let field = |label: &str, value: &str, color: Color| {
        Line::from(vec![
            Span::styled(format!(" {label:<11}"), Style::default().fg(GRAY)),
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
    keys.extend(key("⏎", tr("leave on the prompt", "dejar en el prompt"), GREEN));
    if info.can_edit {
        keys.extend(key("e", tr("edit", "editar"), YEL));
    }
    keys.extend(key("esc", tr("cancel", "cancelar"), GRAY));

    let mut lines = vec![
        field(tr("Command:", "Comando:"), &info.command, FG),
        field(tr("Effect:", "Efecto:"), &info.effect, RED),
    ];
    if let Some(device) = &info.device {
        lines.push(field(tr("Device:", "Dispositivo:"), device, YEL));
    }
    lines.push(field(
        tr("Signals:", "Señales:"),
        &format!("{} {}", tr("fixed rule", "regla fija"), info.signals),
        FG,
    ));
    lines.push(Line::raw(""));
    lines.push(Line::from(keys));
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
    if fill.picking() {
        // a device placeholder: choose among the removable devices, nothing is typed
        let cur = fill.cur;
        let devs = fill.devices.as_mut()?;
        match key.code {
            KeyCode::Up => devs.sel = devs.sel.saturating_sub(1),
            KeyCode::Char('p') if ctrl(&key, 'p') => devs.sel = devs.sel.saturating_sub(1),
            KeyCode::Down if devs.sel + 1 < devs.offered.len() => devs.sel += 1,
            KeyCode::Char('n') if ctrl(&key, 'n') && devs.sel + 1 < devs.offered.len() => devs.sel += 1,
            KeyCode::Enter => {
                fill.values[cur] = Some(devs.offered[devs.sel].path.clone());
                return fill.advance(origin);
            }
            // leave it as `{{placeholder}}`, to be dealt with on the prompt
            KeyCode::Tab => return fill.advance(origin),
            KeyCode::BackTab if cur > 0 => {
                fill.cur -= 1;
                fill.input = fill.values[fill.cur].clone().unwrap_or_default();
                if fill.is_device[fill.cur] {
                    fill.values[fill.cur] = None;
                }
            }
            _ => {}
        }
        return None;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Tab => {
            let v = std::mem::take(&mut fill.input);
            fill.values[fill.cur] = if key.code == KeyCode::Tab || v.is_empty() {
                None
            } else {
                Some(v)
            };
            return fill.advance(origin);
        }
        KeyCode::BackTab if fill.cur > 0 => {
            fill.cur -= 1;
            fill.input = fill.values[fill.cur].clone().unwrap_or_default();
            if fill.is_device[fill.cur] {
                // back on a device: choose again
                fill.values[fill.cur] = None;
            }
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
            Constraint::Length(fill_top_height(app)),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(area);
        draw_fill(f, app, top);
        draw_list(f, app, preview);
        f.render_widget(
            Paragraph::new(Line::styled(fill_hint(app), Style::default().fg(GRAY))),
            footer,
        );
        return;
    }
    if compact {
        draw_compact(f, app, area);
        return;
    }
    let [top, list, detail, footer] = Layout::vertical([
        Constraint::Length(fill_top_height(app)),
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

    let hint = if app.fill.is_some() {
        fill_hint(app)
    } else {
        footer_hint(false)
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, Style::default().fg(GRAY))),
        footer,
    );
}

/// Key hints while filling in placeholders; different while a device is being chosen.
fn fill_hint(app: &App) -> &'static str {
    if app.fill.as_ref().is_some_and(Fill::picking) {
        tr(
            " ↑↓ move · ⏎ choose · tab leave unfilled · shift-tab back · esc insert as is · ctrl-c cancel",
            " ↑↓ mover · ⏎ elegir · tab dejar sin rellenar · shift-tab volver · esc insertar así · ctrl-c cancelar",
        )
    } else {
        footer_hint(true)
    }
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

/// Height of the box at the top of the fill-in: the input line, or the list of devices.
fn fill_top_height(app: &App) -> u16 {
    match app.fill.as_ref().filter(|f| f.picking()) {
        Some(fill) => {
            let devs = fill
                .devices
                .as_ref()
                .map_or(0, |d| d.offered.len().min(DEVICE_ROWS));
            let note = u16::from(fill.devices.as_ref().is_some_and(|d| !d.hidden.is_empty()));
            2 + devs as u16 + note
        }
        None => 3,
    }
}

fn draw_device_picker(f: &mut Frame, fill: &Fill, area: Rect) {
    let Some(devs) = &fill.devices else { return };
    let title = format!(" {} (lsblk) ", tr("device", "dispositivo"));
    let b = Block::bordered()
        .border_style(Style::default().fg(ORANGE))
        .title(Span::styled(
            title,
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD),
        ));
    let inner = b.inner(area);
    f.render_widget(b, area);

    // a window of the list that keeps the selection in view
    let start = devs.sel.saturating_sub(DEVICE_ROWS - 1);
    let mut lines: Vec<Line> = devs
        .offered
        .iter()
        .enumerate()
        .skip(start)
        .take(DEVICE_ROWS)
        .map(|(i, d)| {
            let selected = i == devs.sel;
            let bg = |style: Style| if selected { style.bg(SEL) } else { style };
            let mut spans = vec![
                Span::styled(if selected { "▌ " } else { "  " }, bg(Style::default().fg(MAG))),
                Span::styled(
                    d.path.clone(),
                    bg(Style::default().fg(FG).add_modifier(Modifier::BOLD)),
                ),
            ];
            let rest: Vec<String> = [
                d.model.clone(),
                Some(d.size.clone()),
                d.fstype.clone(),
                d.mountpoint.clone(),
            ]
            .into_iter()
            .flatten()
            .collect();
            spans.push(Span::styled(
                format!("  {}", rest.join("  ")),
                bg(Style::default().fg(GRAY)),
            ));
            spans.push(Span::styled(
                format!("  {}", tr("removable", "extraíble")),
                bg(Style::default().fg(GREEN)),
            ));
            let mut line = Line::from(spans);
            if selected {
                line = line.style(Style::default().bg(SEL));
            }
            line
        })
        .collect();
    if !devs.hidden.is_empty() {
        lines.push(Line::styled(
            format!(
                "  {} {}",
                devs.hidden.join(" / "),
                tr("hidden: not removable", "ocultos: no extraíbles")
            ),
            Style::default().fg(GRAY),
        ));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_fill(f: &mut Frame, app: &App, area: Rect) {
    let fill = app.fill.as_ref().unwrap();
    if fill.picking() {
        draw_device_picker(f, fill, area);
        return;
    }
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

    fn choice(key: KeyEvent, can_install: bool, has_alternative: bool) -> &'static str {
        match install_key(key, can_install, has_alternative) {
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
        assert_eq!(choice(press(KeyCode::Char('y'), none), true, false), "install");
        assert_eq!(choice(press(KeyCode::Char('s'), none), true, false), "install");
        // Enter, Space, Tab and anything else do nothing: the box stays up
        for code in [
            KeyCode::Enter,
            KeyCode::Char(' '),
            KeyCode::Tab,
            KeyCode::Char('x'),
            KeyCode::Down,
        ] {
            assert_eq!(choice(press(code, none), true, true), "nothing", "{code:?}");
        }
        // Ctrl-Y is not Y
        assert_eq!(
            choice(press(KeyCode::Char('y'), KeyModifiers::CONTROL), true, false),
            "nothing"
        );
    }

    #[test]
    fn the_box_can_be_cancelled_and_offers_only_what_exists() {
        let none = KeyModifiers::NONE;
        assert_eq!(choice(press(KeyCode::Esc, none), true, false), "cancel");
        assert_eq!(choice(press(KeyCode::Char('n'), none), false, false), "cancel");
        assert_eq!(
            choice(press(KeyCode::Char('c'), KeyModifiers::CONTROL), true, true),
            "cancel"
        );
        assert_eq!(choice(press(KeyCode::Char('a'), none), true, true), "alternative");
        assert_eq!(
            choice(press(KeyCode::Char('a'), none), true, false),
            "nothing",
            "no alternative to pick"
        );
        // no package for it: `y` must not start an install there is no plan for
        assert_eq!(choice(press(KeyCode::Char('y'), none), false, true), "nothing");
        assert_eq!(
            choice(press(KeyCode::Char('a'), none), false, true),
            "alternative"
        );
    }

    #[test]
    fn the_box_grows_with_what_it_offers() {
        let mut info = InstallBox {
            command: "tree".into(),
            system: "Ubuntu".into(),
            install: None,
            missing: vec!["tree".into()],
            alternative: None,
        };
        let bare = install_box_rows(&info);
        info.install = Some("sudo apt install -y tree".into());
        assert_eq!(install_box_rows(&info), bare + 1);
        info.alternative = Some("find . | sort".into());
        assert_eq!(install_box_rows(&info), bare + 2);
    }

    fn risk_choice(key: KeyEvent, can_edit: bool) -> &'static str {
        match risk_key(key, can_edit) {
            Some(RiskChoice::Leave) => "leave",
            Some(RiskChoice::Edit) => "edit",
            Some(RiskChoice::Cancel) => "cancel",
            None => "nothing",
        }
    }

    /// In this box Enter is the safe key: leaving a command on the prompt runs nothing.
    #[test]
    fn the_risk_box_keys() {
        let none = KeyModifiers::NONE;
        assert_eq!(risk_choice(press(KeyCode::Enter, none), true), "leave");
        assert_eq!(risk_choice(press(KeyCode::Char('e'), none), true), "edit");
        assert_eq!(
            risk_choice(press(KeyCode::Char('e'), none), false),
            "nothing",
            "multiline: no editing"
        );
        assert_eq!(risk_choice(press(KeyCode::Esc, none), true), "cancel");
        assert_eq!(risk_choice(press(KeyCode::Char('n'), none), true), "cancel");
        assert_eq!(
            risk_choice(press(KeyCode::Char('c'), KeyModifiers::CONTROL), true),
            "cancel"
        );
        for code in [
            KeyCode::Char('y'),
            KeyCode::Char(' '),
            KeyCode::Tab,
            KeyCode::Char('x'),
        ] {
            assert_eq!(risk_choice(press(code, none), true), "nothing", "{code:?}");
        }
    }

    #[test]
    fn the_risk_box_grows_with_the_device_row() {
        let mut info = RiskBox {
            command: "x".into(),
            effect: "y".into(),
            device: None,
            signals: "mkfs".into(),
            can_edit: true,
        };
        let bare = risk_box_rows(&info);
        info.device = Some("/dev/sdb1".into());
        assert_eq!(risk_box_rows(&info), bare + 1);
    }

    const DISKS: &str = r#"{"blockdevices":[
      {"name":"sda","model":"Samsung SSD","size":"500G","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/sda",
       "children":[{"name":"sda1","model":null,"size":"500G","fstype":"ext4","mountpoint":"/","rm":false,"path":"/dev/sda1"}]},
      {"name":"nvme0n1","model":"WD Black","size":"1T","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/nvme0n1"},
      {"name":"sdb","model":"SanDisk Ultra","size":"29.8G","fstype":null,"mountpoint":null,"rm":true,"path":"/dev/sdb",
       "children":[{"name":"sdb1","model":null,"size":"29.8G","fstype":"vfat","mountpoint":"/media/pk/USB","rm":true,"path":"/dev/sdb1"}]}
    ]}"#;

    fn device_app<'a>(entries: &'a [Entry], template: &str, json: &str) -> App<'a> {
        let all = devices::parse_lsblk(json).unwrap();
        let mut a = app(entries, ListMode::Explore);
        a.fill = Some(Fill::with_devices(template, Some(&all)));
        a
    }

    fn press_key(a: &mut App, code: KeyCode) -> Option<Outcome> {
        on_fill_key(a, KeyEvent::new(code, KeyModifiers::NONE))
    }

    /// The GIF case: of every disk on the machine only the USB stick can be picked, and the
    /// device goes into the command without typing it.
    #[test]
    fn a_device_placeholder_offers_only_the_removable_device() {
        let e = entries();
        let mut a = device_app(&e, "sudo mkfs.ext4 {{/dev/sdX}}", DISKS);
        let fill = a.fill.as_ref().unwrap();
        assert!(fill.picking());
        let devs = fill.devices.as_ref().unwrap();
        assert_eq!(
            devs.offered.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(),
            vec!["/dev/sdb1"],
            "not sda, nvme0n1 or their partitions"
        );
        assert_eq!(devs.hidden, vec!["sda", "nvme0n1"]);
        // typing does nothing while the list is up
        assert!(press_key(&mut a, KeyCode::Char('x')).is_none());
        assert_eq!(a.fill.as_ref().unwrap().input, "");
        match press_key(&mut a, KeyCode::Enter) {
            Some(Outcome::Selected(cmd, _)) => assert_eq!(cmd, "sudo mkfs.ext4 /dev/sdb1"),
            _ => panic!("expected the command with the device filled in"),
        }
    }

    #[test]
    fn the_device_list_moves_and_can_be_left_unfilled() {
        let two = DISKS.replace(r#""children":[{"name":"sdb1""#, r#""children":[{"name":"sdb2","model":null,"size":"1G","fstype":"ext4","mountpoint":null,"rm":true,"path":"/dev/sdb2"},{"name":"sdb1""#);
        let e = entries();
        let mut a = device_app(&e, "dd if=x.iso of={{device}}", &two);
        assert_eq!(
            a.fill.as_ref().unwrap().devices.as_ref().unwrap().offered.len(),
            2
        );
        press_key(&mut a, KeyCode::Down);
        press_key(&mut a, KeyCode::Down); // at the end: stays
        assert_eq!(a.fill.as_ref().unwrap().devices.as_ref().unwrap().sel, 1);
        press_key(&mut a, KeyCode::Up);
        assert_eq!(a.fill.as_ref().unwrap().devices.as_ref().unwrap().sel, 0);
        // Tab leaves the placeholder as it is
        match press_key(&mut a, KeyCode::Tab) {
            Some(Outcome::Selected(cmd, _)) => assert_eq!(cmd, "dd if=x.iso of={{device}}"),
            _ => panic!("expected the command with the placeholder left"),
        }
    }

    #[test]
    fn with_nothing_removable_the_user_types_the_device() {
        let only_system = r#"{"blockdevices":[{"name":"sda","model":"x","size":"1T","fstype":null,"mountpoint":null,"rm":false,"path":"/dev/sda"}]}"#;
        let e = entries();
        let mut a = device_app(&e, "mkfs.ext4 {{dispositivo}}", only_system);
        assert!(!a.fill.as_ref().unwrap().picking(), "an empty list is no list");
        press_key(&mut a, KeyCode::Char('/'));
        assert_eq!(a.fill.as_ref().unwrap().input, "/");
    }

    #[test]
    fn other_placeholders_are_typed_as_always_and_never_ask_lsblk() {
        let e = entries();
        // the plain constructor must not call lsblk for a template with no device in it
        let fill = Fill::new("tar xf {{file}} --directory {{dir}}");
        assert!(fill.devices.is_none());
        assert!(!fill.picking());
        let mut a = device_app(&e, "ssh {{user}}@{{host}}", DISKS);
        assert!(!a.fill.as_ref().unwrap().picking());
        press_key(&mut a, KeyCode::Char('r'));
        assert_eq!(a.fill.as_ref().unwrap().input, "r");
    }

    #[test]
    fn a_device_after_a_text_placeholder_is_chosen_in_its_turn() {
        let e = entries();
        let mut a = device_app(&e, "dd if={{file}} of={{/dev/sdX}}", DISKS);
        assert!(!a.fill.as_ref().unwrap().picking(), "first the file");
        for c in "a.iso".chars() {
            press_key(&mut a, KeyCode::Char(c));
        }
        assert!(press_key(&mut a, KeyCode::Enter).is_none());
        assert!(a.fill.as_ref().unwrap().picking(), "now the device");
        match press_key(&mut a, KeyCode::Enter) {
            Some(Outcome::Selected(cmd, _)) => assert_eq!(cmd, "dd if=a.iso of=/dev/sdb1"),
            _ => panic!("expected both filled"),
        }
        // going back to the device chooses it again
        let mut b = device_app(&e, "dd if={{file}} of={{/dev/sdX}}", DISKS);
        for c in "a.iso".chars() {
            press_key(&mut b, KeyCode::Char(c));
        }
        press_key(&mut b, KeyCode::Enter);
        press_key(&mut b, KeyCode::BackTab);
        assert_eq!(b.fill.as_ref().unwrap().input, "a.iso");
    }

    #[test]
    fn the_device_box_is_as_tall_as_its_list() {
        let e = entries();
        let a = device_app(&e, "mkfs.ext4 {{/dev/sdX}}", DISKS);
        // 2 borders + one removable device + the note about the hidden disks
        assert_eq!(fill_top_height(&a), 4);
        let b = device_app(&e, "echo {{x}}", DISKS);
        assert_eq!(fill_top_height(&b), 3, "the usual one-line input");
    }

    #[test]
    fn long_titles_are_clipped_with_an_ellipsis() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
        assert_eq!(clip("anything", 0), "…");
    }
}
