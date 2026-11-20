//! The question line of `Ctrl-Space`: it takes over the prompt line on `/dev/tty` (no alternate
//! screen, no box), reads the question, and gives the line back clean so zle can redraw it.
//!
//! Everything is drawn on the tty and nothing is written to stdout, which carries only the
//! result for the widget.

use crate::i18n::tr;
use anyhow::{Context, Result};
use ratatui::crossterm::cursor::MoveToColumn;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::queue;
use ratatui::crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use ratatui::crossterm::terminal::{self, disable_raw_mode, enable_raw_mode, Clear, ClearType};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

// Palette shared with tui.rs (Tokyo Night)
const MAG: Color = Color::Rgb {
    r: 0xbb,
    g: 0x9a,
    b: 0xf7,
};
const DARK: Color = Color::Rgb {
    r: 0x1a,
    g: 0x1b,
    b: 0x26,
};
const GRAY: Color = Color::Rgb {
    r: 0x56,
    g: 0x5f,
    b: 0x89,
};

/// The spinner only shows if resolving takes longer than this, so a fast answer does not flicker.
const SPINNER_DELAY: Duration = Duration::from_millis(150);
const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub enum Answer {
    Question(String),
    /// `Tab` on an empty line: open the list of everything (cheats, history, tldr).
    Explore,
    Cancelled,
}

fn tty() -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context(tr("could not open /dev/tty", "no pude abrir /dev/tty"))
}

struct RawMode;

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

/// Number of terminal cells, counting each char as one (enough for Latin text).
fn cells(s: &str) -> usize {
    s.chars().count()
}

/// The last `max` characters of `s`, so a long question keeps its end (where the cursor is) in view.
fn tail(s: &str, max: usize) -> String {
    let n = cells(s);
    s.chars().skip(n.saturating_sub(max)).collect()
}

fn badge() -> &'static str {
    tr(" ✦ ask ", " ✦ pregunta ")
}

fn hint() -> &'static str {
    tr(
        "⏎ ask · tab explore · esc exit",
        "⏎ preguntar · tab explorar · esc salir",
    )
}

/// Redraws the whole line: badge, question, and the hint on the right when there is room.
fn draw(out: &mut impl Write, text: &str, width: usize) -> Result<()> {
    let badge = badge();
    let room = width.saturating_sub(cells(badge) + 2);
    let shown = tail(text, room);
    queue!(out, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    queue!(
        out,
        SetBackgroundColor(MAG),
        SetForegroundColor(DARK),
        SetAttribute(Attribute::Bold),
        Print(badge),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print(' '),
        Print(&shown)
    )?;
    let cursor_col = cells(badge) + 1 + cells(&shown);
    let hint = hint();
    if cursor_col + 2 + cells(hint) <= width {
        queue!(
            out,
            MoveToColumn((width - cells(hint)) as u16),
            SetForegroundColor(GRAY),
            Print(hint),
            ResetColor
        )?;
    }
    queue!(out, MoveToColumn(cursor_col as u16))?;
    out.flush()?;
    Ok(())
}

fn clear_line(out: &mut impl Write) -> Result<()> {
    queue!(out, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    out.flush()?;
    Ok(())
}

/// Reads the question on the current line. `initial` pre-fills it (the text that was already on
/// the prompt), editable. The line is left empty on return, whatever the outcome.
pub fn read_question(initial: &str) -> Result<Answer> {
    let mut out = tty()?;
    enable_raw_mode()?;
    let _raw = RawMode;
    let mut text = initial.replace('\n', " ");

    let answer = loop {
        let width = terminal::size().map(|(w, _)| w as usize).unwrap_or(80);
        draw(&mut out, &text, width)?;
        let Event::Key(key) = event::read()? else { continue };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter if !text.trim().is_empty() => break Answer::Question(text.trim().to_string()),
            KeyCode::Esc => break Answer::Cancelled,
            KeyCode::Char('c' | 'g') if ctrl => break Answer::Cancelled,
            KeyCode::Tab if text.trim().is_empty() => break Answer::Explore,
            KeyCode::Backspace => {
                text.pop();
            }
            KeyCode::Char('u') if ctrl => text.clear(),
            KeyCode::Char('w') if ctrl => {
                let keep = text.trim_end().rfind(' ').map_or(0, |i| i + 1);
                text.truncate(keep);
            }
            KeyCode::Char(c) if !ctrl => text.push(c),
            _ => {}
        }
    };
    clear_line(&mut out)?;
    Ok(answer)
}

/// Shows a one-line message on the current line and waits for a key. For the tmux popup, which
/// closes when the process ends and would otherwise swallow the message.
pub fn notice(message: &str) -> Result<()> {
    let mut out = tty()?;
    enable_raw_mode()?;
    let _raw = RawMode;
    queue!(
        out,
        MoveToColumn(0),
        Clear(ClearType::CurrentLine),
        Print(message),
        SetForegroundColor(GRAY),
        Print(format!("  {}", tr("(press any key)", "(pulsa una tecla)"))),
        ResetColor
    )?;
    out.flush()?;
    loop {
        if let Event::Key(key) = event::read()? {
            if key.kind == KeyEventKind::Press {
                break;
            }
        }
    }
    clear_line(&mut out)
}

/// Runs `work` and, if it takes longer than `SPINNER_DELAY`, shows `resolving…` with a spinner
/// on the prompt line meanwhile. The line is left empty afterwards.
pub fn with_spinner<T>(work: impl FnOnce() -> T) -> Result<T> {
    let mut out = tty()?;
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            let start = Instant::now();
            let mut frame = 0;
            while !done.load(Ordering::Relaxed) {
                if start.elapsed() >= SPINNER_DELAY {
                    let _ = queue!(
                        out,
                        MoveToColumn(0),
                        Clear(ClearType::CurrentLine),
                        SetBackgroundColor(MAG),
                        SetForegroundColor(DARK),
                        Print(badge()),
                        ResetColor,
                        SetForegroundColor(GRAY),
                        Print(format!(
                            " {} {}",
                            SPINNER_FRAMES[frame % SPINNER_FRAMES.len()],
                            tr("resolving…", "resolviendo…")
                        )),
                        ResetColor
                    );
                    let _ = out.flush();
                    frame += 1;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            let _ = clear_line(&mut out);
        });
        let value = work();
        done.store(true, Ordering::Relaxed);
        value
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_questions_keep_their_end_in_view() {
        assert_eq!(tail("hello world", 5), "world");
        assert_eq!(tail("short", 20), "short");
        assert_eq!(tail("ñandú rápido", 6), "rápido");
        assert_eq!(tail("abc", 0), "");
    }

    #[test]
    fn the_hint_only_shows_when_it_fits() {
        let mut wide = Vec::new();
        draw(&mut wide, "hola", 80).unwrap();
        assert!(String::from_utf8_lossy(&wide).contains("esc exit"));
        let mut narrow = Vec::new();
        draw(&mut narrow, "hola", 20).unwrap();
        assert!(!String::from_utf8_lossy(&narrow).contains("esc exit"));
    }

    #[test]
    fn drawing_never_overflows_the_width() {
        let mut out = Vec::new();
        let long = "x".repeat(300);
        draw(&mut out, &long, 40).unwrap();
        let s = String::from_utf8_lossy(&out);
        assert_eq!(s.matches('x').count(), 40 - cells(badge()) - 2);
    }
}
