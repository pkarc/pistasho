//! Searcher interface. Draws on `/dev/tty` so stdout stays free
//! and the zsh widget can capture the chosen command.

use crate::i18n::tr;
use crate::index::{Entry, Source};
use crate::placeholders::{self, Part};
use crate::search::Searcher;
use anyhow::{Context, Result};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::{Frame, Terminal};
use std::fs::File;

const MAX_RESULTS: usize = 300;

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
    Selected(String),
    Cancelled,
}

pub struct Options {
    pub initial_query: String,
    pub fill: bool,
}

struct Fill {
    template: String,
    names: Vec<String>,
    values: Vec<Option<String>>,
    cur: usize,
    input: String,
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
}

impl<'a> App<'a> {
    fn refresh(&mut self) {
        self.results = self.searcher.rank(self.entries, &self.query, MAX_RESULTS);
        self.sel = 0;
        self.offset = 0;
    }

    fn selected(&self) -> Option<&'a Entry> {
        self.results.get(self.sel).map(|&i| &self.entries[i])
    }
}

/// Restores the terminal even on a panic.
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        if let Ok(mut tty) = File::options().write(true).open("/dev/tty") {
            let _ = execute!(tty, LeaveAlternateScreen);
        }
    }
}

pub fn run(entries: &[Entry], opts: Options) -> Result<Outcome> {
    let tty = File::options()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context(tr("could not open /dev/tty", "no pude abrir /dev/tty"))?;
    let mut tty_w = tty.try_clone()?;
    enable_raw_mode()?;
    let _guard = Guard;
    execute!(tty_w, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(tty))?;

    let mut app = App {
        entries,
        searcher: Searcher::new(entries),
        query: opts.initial_query,
        results: Vec::new(),
        sel: 0,
        offset: 0,
        fill: None,
        fill_enabled: opts.fill,
    };
    app.refresh();

    loop {
        terminal.draw(|f| draw(f, &mut app))?;
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
            terminal.show_cursor()?;
            return Ok(outcome);
        }
    }
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
                let names = placeholders::names(&entry.cmd);
                app.fill = Some(Fill {
                    template: entry.cmd.clone(),
                    values: vec![None; names.len()],
                    names,
                    cur: 0,
                    input: String::new(),
                });
                None
            } else {
                Some(Outcome::Selected(entry.cmd.clone()))
            }
        }
        // Tab: insert the template as is, with the {{placeholders}}
        KeyCode::Tab => app.selected().map(|e| Outcome::Selected(e.cmd.clone())),
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
    let fill = app.fill.as_mut()?;
    // Esc: insert with the placeholders still missing
    if key.code == KeyCode::Esc || ctrl(&key, 'g') {
        return Some(Outcome::Selected(placeholders::fill(
            &fill.template,
            &fill.values,
        )));
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
                return Some(Outcome::Selected(placeholders::fill(
                    &fill.template,
                    &fill.values,
                )));
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

    let hint = if app.fill.is_some() {
        tr(
            " ⏎ next · tab skip · shift-tab back · esc insert as is · ctrl-c cancel",
            " ⏎ siguiente · tab saltar · shift-tab volver · esc insertar así · ctrl-c cancelar",
        )
    } else {
        tr(
            " ↑↓ move · ⏎ insert · tab without filling · ctrl-u clear · esc quit",
            " ↑↓ mover · ⏎ insertar · tab sin rellenar · ctrl-u borrar · esc salir",
        )
    };
    f.render_widget(
        Paragraph::new(Line::styled(hint, Style::default().fg(GRAY))),
        footer,
    );
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

    if app.results.is_empty() {
        let msg = if app.entries.is_empty() {
            tr(
                " Empty index: run `pistasho update`",
                " Índice vacío: ejecuta `pistasho update`",
            )
        } else {
            tr(" no results", " sin resultados")
        };
        f.render_widget(
            Paragraph::new(Line::styled(msg, Style::default().fg(GRAY))),
            inner,
        );
        return;
    }

    // scroll so the selection is always visible
    if app.sel < app.offset {
        app.offset = app.sel;
    } else if app.sel >= app.offset + h {
        app.offset = app.sel + 1 - h;
    }

    let lines: Vec<Line> = app
        .results
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
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
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
