//! The built-in terminal panel: a shell in a pseudo terminal (ConPTY on
//! Windows), emulated by `alacritty_terminal` and drawn with egui.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Notifier};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use alacritty_terminal::Term;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, EventFilter, FontId, Key, Modifiers, Sense, Ui};

const FONT_SIZE: f32 = 13.0;
const SCROLLBACK: usize = 5000;

#[derive(Clone)]
struct Listener {
    tx: Sender<Event>,
    ctx: egui::Context,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    notifier: Notifier,
    events: Receiver<Event>,
    /// Columns and lines the terminal currently has.
    size: (usize, usize),
    /// Mouse wheel movement not yet turned into whole lines.
    scroll_rest: f32,
    /// The shell has ended.
    pub exited: bool,
    pub title: String,
    /// Is the shell `cmd.exe` (it needs `cd /d` to change drives)?
    is_cmd: bool,
    /// Take the keyboard focus on the next frame.
    pub focus_next: bool,
}

/// The command for `shell` ("" = the system's default shell).
fn shell_command(shell: &str) -> (String, Vec<String>) {
    let shell = shell.trim();
    if !shell.is_empty() {
        let mut parts = shell.split_whitespace().map(str::to_owned);
        let program = parts.next().unwrap_or_default();
        return (program, parts.collect());
    }
    if cfg!(windows) {
        ("powershell.exe".into(), vec!["-NoLogo".into()])
    } else {
        let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        (sh, Vec::new())
    }
}

impl Terminal {
    /// Start `shell` in `dir`.
    pub fn spawn(ctx: &egui::Context, dir: &Path, shell: &str) -> io::Result<Self> {
        let (program, args) = shell_command(shell);
        let is_cmd = Path::new(&program)
            .file_stem()
            .is_some_and(|s| s.eq_ignore_ascii_case("cmd"));
        let options = tty::Options {
            shell: Some(tty::Shell::new(program, args)),
            working_directory: Some(dir.to_path_buf()),
            drain_on_exit: true,
            env: HashMap::from([
                ("TERM".to_owned(), "xterm-256color".to_owned()),
                ("COLORTERM".to_owned(), "truecolor".to_owned()),
            ]),
            #[cfg(windows)]
            escape_args: true,
        };
        let (cols, lines) = (80, 24);
        let window = WindowSize {
            num_lines: lines as u16,
            num_cols: cols as u16,
            cell_width: 8,
            cell_height: 16,
        };
        let pty = tty::new(&options, window, 0)?;
        let (tx, events) = mpsc::channel();
        let listener = Listener {
            tx,
            ctx: ctx.clone(),
        };
        let config = Config {
            scrolling_history: SCROLLBACK,
            ..Default::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &TermSize::new(cols, lines),
            listener.clone(),
        )));
        let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();
        Ok(Self {
            term,
            notifier,
            events,
            size: (cols, lines),
            scroll_rest: 0.0,
            exited: false,
            focus_next: false,
            title: String::new(),
            is_cmd,
        })
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.notifier.notify(bytes);
    }

    /// Type a `cd` to `dir` into the shell.
    pub fn cd(&self, dir: &Path) {
        let dir = dir.display().to_string();
        let line = if self.is_cmd {
            format!("cd /d \"{dir}\"\r")
        } else if cfg!(windows) {
            format!("cd '{}'\r", dir.replace('\'', "''"))
        } else {
            format!("cd '{}'\r", dir.replace('\'', r"'\''"))
        };
        self.write(line.into_bytes());
    }

    fn poll(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Event::Title(t) => self.title = t,
                Event::ResetTitle => self.title.clear(),
                Event::PtyWrite(text) => self.write(text.into_bytes()),
                Event::Exit | Event::ChildExit(_) => self.exited = true,
                _ => {}
            }
        }
    }

    /// Draw the terminal into the rest of `ui` and handle its input.
    /// Returns whether it has the keyboard focus.
    pub fn ui(&mut self, ui: &mut Ui) -> bool {
        self.poll();
        let font = FontId::monospace(FONT_SIZE);
        let (cell_w, cell_h) = ui.fonts_mut(|f| (f.glyph_width(&font, 'M'), f.row_height(&font)));
        let rect = ui.available_rect_before_wrap();
        let cols = (((rect.width() - 8.0) / cell_w).floor() as usize).max(2);
        let lines = (((rect.height() - 4.0) / cell_h).floor() as usize).max(1);
        if (cols, lines) != self.size {
            self.size = (cols, lines);
            self.term.lock().resize(TermSize::new(cols, lines));
            self.notifier.on_resize(WindowSize {
                num_lines: lines as u16,
                num_cols: cols as u16,
                cell_width: cell_w as u16,
                cell_height: cell_h as u16,
            });
        }

        let id = egui::Id::unique("terminal_view");
        let resp = ui.interact(rect, id, Sense::click());
        if resp.clicked() || std::mem::take(&mut self.focus_next) {
            resp.request_focus();
        }
        let focused = resp.has_focus();
        if focused {
            // Keep Tab, arrows and Esc for the shell.
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                )
            });
            self.input(ui);
        }
        if resp.hovered() {
            self.scroll_rest += ui.input(|i| i.smooth_scroll_delta.y);
            let whole = (self.scroll_rest / cell_h).trunc();
            if whole != 0.0 {
                self.scroll_rest -= whole * cell_h;
                self.term.lock().scroll_display(Scroll::Delta(whole as i32));
            }
        }

        let visuals = ui.visuals();
        let default_fg = visuals.text_color();
        let default_bg = visuals.extreme_bg_color;
        let accent = visuals.selection.bg_fill;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, default_bg);

        let term = self.term.lock();
        let content = term.renderable_content();
        let offset = content.display_offset as i32;
        let blank = (' ', default_fg, Color32::TRANSPARENT);
        let mut grid = vec![vec![blank; cols]; lines];
        for cell in content.display_iter {
            let line = cell.point.line.0 + offset;
            let col = cell.point.column.0;
            if line < 0 || line as usize >= lines || col >= cols {
                continue;
            }
            let flags = cell.flags;
            // Bold text in one of the 8 basic colours uses its bright variant.
            let fg = match cell.fg {
                Color::Named(n) if flags.contains(Flags::BOLD) && (n as usize) < 8 => {
                    Color::Indexed(n as u8 + 8)
                }
                c => c,
            };
            let mut fg = resolve(fg, content.colors, default_fg, default_bg);
            let mut bg = resolve(cell.bg, content.colors, default_fg, default_bg);
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            if flags.contains(Flags::DIM) {
                fg = fg.gamma_multiply(0.66);
            }
            let c = if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::HIDDEN) {
                ' '
            } else {
                cell.c
            };
            let bg = if bg == default_bg {
                Color32::TRANSPARENT
            } else {
                bg
            };
            grid[line as usize][col] = (c, fg, bg);
        }
        let cursor = content.cursor;
        drop(term);

        let origin = rect.min + egui::vec2(4.0, 2.0);
        for (i, row) in grid.iter().enumerate() {
            let mut job = LayoutJob::default();
            let mut run = String::new();
            let mut style = (row[0].1, row[0].2);
            for &(c, fg, bg) in row {
                if (fg, bg) != style {
                    push_run(&mut job, &mut run, style, &font);
                    style = (fg, bg);
                }
                run.push(c);
            }
            push_run(&mut job, &mut run, style, &font);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            painter.galley(
                origin + egui::vec2(0.0, i as f32 * cell_h),
                galley,
                default_fg,
            );
        }

        let line = cursor.point.line.0 + offset;
        if cursor.shape != CursorShape::Hidden && line >= 0 && (line as usize) < lines {
            let pos =
                origin + egui::vec2(cursor.point.column.0 as f32 * cell_w, line as f32 * cell_h);
            let r = egui::Rect::from_min_size(pos, egui::vec2(cell_w, cell_h));
            if focused {
                painter.rect_filled(r, 1.0, accent.gamma_multiply(0.7));
            } else {
                painter.rect_stroke(
                    r,
                    1.0,
                    egui::Stroke::new(1.0, default_fg.gamma_multiply(0.6)),
                    egui::StrokeKind::Inside,
                );
            }
        }
        ui.allocate_rect(rect, Sense::hover());
        focused
    }

    /// Turn typed text and keys into bytes for the shell.
    fn input(&mut self, ui: &Ui) {
        let mode = *self.term.lock().mode();
        let app_cursor = mode.contains(TermMode::APP_CURSOR);
        let events = ui.input(|i| i.events.clone());
        let mut typed = false;
        for event in events {
            match event {
                egui::Event::Text(text) => {
                    self.write(text.into_bytes());
                    typed = true;
                }
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => {
                    if modifiers.matches_exact(Modifiers::COMMAND | Modifiers::SHIFT)
                        && key == Key::V
                    {
                        self.paste(mode);
                        typed = true;
                    } else if let Some(bytes) = key_bytes(key, modifiers, app_cursor) {
                        self.write(bytes);
                        typed = true;
                    }
                }
                _ => {}
            }
        }
        if typed {
            // Typing jumps back to the bottom like any terminal.
            self.term.lock().scroll_display(Scroll::Bottom);
        }
    }

    fn paste(&self, mode: TermMode) {
        let Ok(text) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) else {
            return;
        };
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if mode.contains(TermMode::BRACKETED_PASTE) {
            self.write(format!("\x1b[200~{text}\x1b[201~").into_bytes());
        } else {
            self.write(text.into_bytes());
        }
    }
}

fn push_run(job: &mut LayoutJob, run: &mut String, (fg, bg): (Color32, Color32), font: &FontId) {
    if run.is_empty() {
        return;
    }
    job.append(
        run,
        0.0,
        TextFormat {
            font_id: font.clone(),
            color: fg,
            background: bg,
            ..Default::default()
        },
    );
    run.clear();
}

/// Escape sequences for special keys (xterm style).
fn key_bytes(key: Key, m: Modifiers, app_cursor: bool) -> Option<Cow<'static, [u8]>> {
    let s: &'static str = match key {
        Key::Enter => "\r",
        Key::Backspace => "\x7f",
        Key::Tab if m.shift => "\x1b[Z",
        Key::Tab => "\t",
        Key::Escape => "\x1b",
        Key::ArrowUp if app_cursor => "\x1bOA",
        Key::ArrowDown if app_cursor => "\x1bOB",
        Key::ArrowRight if app_cursor => "\x1bOC",
        Key::ArrowLeft if app_cursor => "\x1bOD",
        Key::ArrowUp => "\x1b[A",
        Key::ArrowDown => "\x1b[B",
        Key::ArrowRight => "\x1b[C",
        Key::ArrowLeft => "\x1b[D",
        Key::Home => "\x1b[H",
        Key::End => "\x1b[F",
        Key::Insert => "\x1b[2~",
        Key::Delete => "\x1b[3~",
        Key::PageUp => "\x1b[5~",
        Key::PageDown => "\x1b[6~",
        Key::F1 => "\x1bOP",
        Key::F2 => "\x1bOQ",
        Key::F3 => "\x1bOR",
        Key::F4 => "\x1bOS",
        Key::F5 => "\x1b[15~",
        Key::F6 => "\x1b[17~",
        Key::F7 => "\x1b[18~",
        Key::F8 => "\x1b[19~",
        Key::F9 => "\x1b[20~",
        Key::F10 => "\x1b[21~",
        Key::F11 => "\x1b[23~",
        Key::F12 => "\x1b[24~",
        _ => {
            // Ctrl+letter → control character (Ctrl+C = 0x03, …).
            if (m.ctrl || m.command) && !m.alt {
                let name = key.name();
                if let [c] = name.as_bytes() {
                    if c.is_ascii_alphabetic() {
                        return Some(Cow::Owned(vec![c.to_ascii_uppercase() - b'@']));
                    }
                }
            }
            return None;
        }
    };
    Some(Cow::Borrowed(s.as_bytes()))
}

/// Windows Terminal's "Campbell" colours for the 16 ANSI colours (with a
/// lighter blue, which reads better on dark backgrounds).
const ANSI: [[u8; 3]; 16] = [
    [0x0c, 0x0c, 0x0c],
    [0xc5, 0x0f, 0x1f],
    [0x13, 0xa1, 0x0e],
    [0xc1, 0x9c, 0x00],
    [0x3b, 0x78, 0xff],
    [0x88, 0x17, 0x98],
    [0x3a, 0x96, 0xdd],
    [0xcc, 0xcc, 0xcc],
    [0x76, 0x76, 0x76],
    [0xe7, 0x48, 0x56],
    [0x16, 0xc6, 0x0c],
    [0xf9, 0xf1, 0xa5],
    [0x6a, 0x9f, 0xff],
    [0xb4, 0x00, 0x9e],
    [0x61, 0xd6, 0xd6],
    [0xf2, 0xf2, 0xf2],
];

fn rgb(c: Rgb) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// The xterm 256-colour palette.
fn indexed(i: u8) -> Color32 {
    match i {
        0..=15 => {
            let [r, g, b] = ANSI[i as usize];
            Color32::from_rgb(r, g, b)
        }
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Color32::from_rgb(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Color32::from_rgb(v, v, v)
        }
    }
}

fn resolve(c: Color, colors: &Colors, fg: Color32, bg: Color32) -> Color32 {
    match c {
        Color::Spec(c) => rgb(c),
        Color::Indexed(i) => colors[i as usize].map(rgb).unwrap_or_else(|| indexed(i)),
        Color::Named(n) => {
            if let Some(c) = colors[n as usize] {
                return rgb(c);
            }
            let i = n as usize;
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => fg,
                NamedColor::DimForeground => fg.gamma_multiply(0.66),
                NamedColor::Background => bg,
                _ if i < 16 => indexed(i as u8),
                // The dim variants follow the 8 normal colours.
                _ => indexed((i - NamedColor::DimBlack as usize) as u8 % 8).gamma_multiply(0.66),
            }
        }
    }
}

/// Where a new terminal starts: the folder, or the home folder.
pub fn start_dir(folder: Option<&Path>) -> PathBuf {
    folder
        .map(Path::to_path_buf)
        .unwrap_or_else(ox_core::places::home_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_escape_sequences() {
        let none = Modifiers::NONE;
        assert_eq!(&*key_bytes(Key::Enter, none, false).unwrap(), b"\r");
        assert_eq!(&*key_bytes(Key::ArrowUp, none, false).unwrap(), b"\x1b[A");
        assert_eq!(&*key_bytes(Key::ArrowUp, none, true).unwrap(), b"\x1bOA");
        assert_eq!(&*key_bytes(Key::C, Modifiers::COMMAND, false).unwrap(), [3]);
        assert!(key_bytes(Key::C, none, false).is_none());
    }

    #[test]
    fn palette() {
        assert_eq!(indexed(1), Color32::from_rgb(0xc5, 0x0f, 0x1f));
        assert_eq!(indexed(16), Color32::from_rgb(0, 0, 0));
        assert_eq!(indexed(231), Color32::from_rgb(255, 255, 255));
        assert_eq!(indexed(232), Color32::from_rgb(8, 8, 8));
    }

    #[test]
    fn shell_command_splits_custom_shells() {
        let (p, a) = shell_command("pwsh -NoLogo -NoProfile");
        assert_eq!(p, "pwsh");
        assert_eq!(a, ["-NoLogo", "-NoProfile"]);
        assert!(!shell_command("").0.is_empty());
    }
}
