//! Custom commands (Settings → Custom commands): running them, the question
//! before, their shortcuts and the windows with their output.

use std::path::PathBuf;

use crossbeam_channel::{Receiver, Sender};
use egui::{Context, KeyboardShortcut};
use ox_core::commands::{run, CustomCommand, Invocation, Target, Window};

use super::*;

/// Lines kept in an output window (the oldest go first).
const MAX_LINES: usize = 20_000;

pub enum CommandAction {
    /// Run command `index` on `items` (none: on the background of `folder`).
    Run {
        index: usize,
        items: Vec<PathBuf>,
        folder: Option<PathBuf>,
    },
    /// The question before running was answered: run (`true`) or not.
    Confirm(bool),
}

/// A run waiting for the user to say yes.
pub struct PendingRun {
    pub command: CustomCommand,
    pub runs: Vec<Invocation>,
}

enum OutputEvent {
    Line(String),
    /// The run for another item starts.
    Item(String),
    /// All runs are done; the first failure, if any.
    Done(Option<String>),
}

/// The output of a command run with "Wait and show the output".
pub struct OutputWindow {
    pub id: u64,
    pub title: String,
    pub lines: Vec<String>,
    /// `None` while it runs; then the failure, if any.
    pub done: Option<Option<String>>,
    pub open: bool,
    rx: Receiver<OutputEvent>,
}

impl OutputWindow {
    /// Take in what the run sent since last frame.
    pub fn poll(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                OutputEvent::Line(l) => self.lines.push(l),
                OutputEvent::Item(name) => self.lines.push(format!("—— {name} ——")),
                OutputEvent::Done(result) => self.done = Some(result),
            }
        }
        if self.lines.len() > MAX_LINES {
            self.lines.drain(..self.lines.len() - MAX_LINES);
        }
    }
}

#[derive(Default)]
pub struct CustomCommands {
    pub confirm: Option<PendingRun>,
    pub outputs: Vec<OutputWindow>,
    next_id: u64,
    /// Settings page: the command whose form is open …
    pub editing: Option<usize>,
    /// … and its shortcut is being recorded (app shortcuts are off then).
    pub recording: bool,
}

impl CustomCommands {
    /// A shortcut is being recorded or a run waits for its question: the
    /// app's shortcuts are off.
    pub fn takes_keys(&self) -> bool {
        self.recording || self.confirm.is_some()
    }
}

/// Whether `a` and `b` are the same modifiers (Ctrl and Command alike).
fn same_modifiers(a: Modifiers, b: Modifiers) -> bool {
    (a.ctrl || a.command) == (b.ctrl || b.command) && a.alt == b.alt && a.shift == b.shift
}

/// The parsed shortcut of `c`, if it has a valid one.
pub fn shortcut_of(c: &CustomCommand) -> Option<KeyboardShortcut> {
    match c.shortcut.trim() {
        "" => None,
        s => crate::shortcuts::parse(s),
    }
}

impl OxApp {
    pub(super) fn command_action(&mut self, a: CommandAction) {
        match a {
            CommandAction::Run {
                index,
                items,
                folder,
            } => {
                let Some(command) = self.settings.commands.0.get(index).cloned() else {
                    return;
                };
                let folder = folder.or_else(|| self.current_folder());
                let runs = command.invocations(&items, folder.as_deref());
                if runs.is_empty() {
                    return;
                }
                if command.confirm {
                    self.commands.confirm = Some(PendingRun { command, runs });
                } else {
                    self.start_runs(command, runs);
                }
            }
            CommandAction::Confirm(yes) => {
                if let Some(p) = self.commands.confirm.take().filter(|_| yes) {
                    self.start_runs(p.command, p.runs);
                }
            }
        }
    }

    fn start_runs(&mut self, command: CustomCommand, runs: Vec<Invocation>) {
        let name = command.name.clone();
        let window = command.window;
        if window != Window::Output {
            self.in_background(move || {
                for inv in &runs {
                    if let Err(f) = run::run(inv, window, &mut |_| {}) {
                        return Notice::Toast(ox_core::trf!(
                            "“{name}” failed: {e}",
                            name = name,
                            e = f.message()
                        ));
                    }
                }
                match window {
                    Window::Hidden => Notice::Toast(ox_core::trf!("“{name}” is done", name = name)),
                    _ => Notice::Nothing,
                }
            });
            return;
        }
        let (tx, rx): (Sender<OutputEvent>, _) = crossbeam_channel::unbounded();
        self.commands.next_id += 1;
        self.commands.outputs.push(OutputWindow {
            id: self.commands.next_id,
            title: name,
            lines: Vec::new(),
            done: None,
            open: true,
            rx,
        });
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let several = runs.len() > 1;
            let mut failure = None;
            for inv in &runs {
                if several {
                    let item = inv.items.first().map(|p| entry::display_name(p));
                    let _ = tx.send(OutputEvent::Item(item.unwrap_or_default()));
                }
                let mut send = |l: String| {
                    let _ = tx.send(OutputEvent::Line(l));
                    ctx.request_repaint();
                };
                if let Err(f) = run::run(inv, Window::Output, &mut send) {
                    let msg = f.message();
                    let _ = tx.send(OutputEvent::Line(format!("⚠ {msg}")));
                    failure.get_or_insert(msg);
                }
            }
            let _ = tx.send(OutputEvent::Done(failure));
            ctx.request_repaint();
        });
    }

    /// The selection of the active tab as (path, is a folder), for custom
    /// commands; `None` where they aren't offered (Recycle Bin, archives,
    /// pages without files).
    pub fn command_selection(&self) -> Option<Vec<(PathBuf, bool)>> {
        let tab = self.tabs.active();
        let location = tab.location();
        if !location.shows_files() || location.is_bin() || location.is_read_only() {
            return None;
        }
        let items: Vec<(PathBuf, bool)> = tab
            .view
            .selected_entries()
            .into_iter()
            .map(|e| (e.path.clone(), e.is_dir))
            .collect();
        // Search results inside archives have no real path.
        let virtual_items = tab
            .view
            .selected_entries()
            .iter()
            .any(|e| e.readonly && archive::split_virtual(&e.path).is_some());
        (!virtual_items).then_some(items)
    }

    /// Custom commands whose shortcut was pressed. Built-in shortcuts win
    /// over them (the settings warn about it), and the modifiers have to
    /// match exactly, so Ctrl+Shift+X doesn't also count as Ctrl+X.
    pub(super) fn command_shortcuts(&mut self, ctx: &Context) {
        let keys: Vec<(usize, KeyboardShortcut)> = self
            .settings
            .commands
            .0
            .iter()
            .enumerate()
            .filter_map(|(i, c)| shortcut_of(c).map(|k| (i, k)))
            .filter(|(_, k)| {
                !crate::shortcuts::COMMANDS
                    .iter()
                    .any(|c| self.keymap.keys(c.cmd).contains(k))
            })
            .collect();
        if keys.is_empty() {
            return;
        }
        let hit = ctx.input_mut(|i| {
            let mut hit = None;
            i.events.retain(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    modifiers,
                    ..
                } if hit.is_none() => {
                    let found = keys.iter().find(|(_, k)| {
                        k.logical_key == *key && same_modifiers(k.modifiers, *modifiers)
                    });
                    hit = found.map(|(i, _)| *i);
                    hit.is_none()
                }
                _ => true,
            });
            hit
        });
        let Some(index) = hit else {
            return;
        };
        let command = &self.settings.commands.0[index];
        let folder = self.current_folder();
        let target = self.command_selection();
        let action = match &target {
            Some(items) if !items.is_empty() && command.applies(Target::Items(items)) => {
                Some(CommandAction::Run {
                    index,
                    items: items.iter().map(|(p, _)| p.clone()).collect(),
                    folder: None,
                })
            }
            Some(items)
                if items.is_empty() && folder.is_some() && command.applies(Target::Background) =>
            {
                Some(CommandAction::Run {
                    index,
                    items: Vec::new(),
                    folder,
                })
            }
            _ => None,
        };
        match action {
            Some(a) => self.push(Action::Command(a)),
            None => {
                let name = command.name.clone();
                self.toast(ox_core::trf!(
                    "“{name}” isn't for what is selected",
                    name = name
                ));
            }
        }
    }
}
