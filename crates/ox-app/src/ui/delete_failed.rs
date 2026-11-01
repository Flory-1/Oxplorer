//! The dialog shown when items couldn't be deleted: the exact error, the
//! programs that have them open (with a button to end those and delete
//! again) and, when access is denied, deleting as administrator.

use std::path::PathBuf;

use egui::{Color32, RichText, Ui};
use ox_core::entry::display_name;
use ox_core::i18n::t as tr;
use ox_core::locks::{self, Locker};

use crate::app::Action;

/// Items that couldn't be deleted.
pub struct DeleteFailed {
    /// What failed (may be files deep inside the selected items).
    pub items: Vec<Failed>,
    /// The selected items the failures are in: trying again deletes these.
    pub retry: Vec<PathBuf>,
    /// They were to go to the Recycle Bin (trying again does the same).
    pub to_trash: bool,
    /// Programs that couldn't be ended.
    pub notes: Vec<String>,
}

pub struct Failed {
    pub path: PathBuf,
    pub error: String,
    pub lockers: Vec<Locker>,
}

impl DeleteFailed {
    fn paths(&self) -> Vec<PathBuf> {
        self.retry.clone()
    }

    /// Everything the dialog says, as text for the clipboard.
    fn details(&self) -> String {
        let mut out = String::new();
        for i in &self.items {
            out.push_str(&format!("{}\n  {}\n", i.path.display(), i.error));
            for l in &i.lockers {
                out.push_str(&format!("  {}\n", locker_text(l)));
            }
        }
        for n in &self.notes {
            out.push_str(n);
            out.push('\n');
        }
        out
    }
}

/// "Word (WINWORD.EXE) · PID 1234".
fn locker_text(l: &Locker) -> String {
    let exe = l
        .exe
        .as_ref()
        .and_then(|e| e.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    let name = match exe {
        Some(exe) if !exe.eq_ignore_ascii_case(&l.name) && !l.name.is_empty() => {
            format!("{} ({exe})", l.name)
        }
        Some(exe) => exe,
        None => l.name.clone(),
    };
    ox_core::trf!("{name} · PID {pid}", name = name, pid = l.pid)
}

/// Draw the dialog; returns whether it stays open.
pub fn show(ui: &mut Ui, d: &DeleteFailed, actions: &mut Vec<Action>) -> bool {
    let mut keep = true;
    let error_color = Color32::from_rgb(0xe5, 0x48, 0x4d);
    ui.heading(match d.items.as_slice() {
        [one] => ox_core::trf!("Couldn't delete \"{v1}\"", v1 = display_name(&one.path)),
        many => ox_core::trf!("Couldn't delete {v1} items", v1 = many.len()),
    });
    ui.add_space(6.0);
    for n in &d.notes {
        ui.label(RichText::new(n).color(error_color));
    }
    let retry = |end: Vec<u32>| Action::RetryDelete {
        paths: d.paths(),
        to_trash: d.to_trash,
        end,
    };
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for item in &d.items {
                ui.label(RichText::new(display_name(&item.path)).strong());
                ui.label(RichText::new(item.path.display().to_string()).weak().small());
                ui.add(
                    egui::Label::new(RichText::new(&item.error).color(error_color))
                        .selectable(true),
                );
                if !item.lockers.is_empty() {
                    ui.add_space(2.0);
                    ui.label(tr("It is open in:"));
                    for l in &item.lockers {
                        ui.horizontal(|ui| {
                            let text = locker_text(l);
                            let label = ui.add(egui::Label::new(&text).selectable(true));
                            if let Some(exe) = &l.exe {
                                label.on_hover_text(exe.display().to_string());
                            }
                            if ui
                                .button(tr("End process and delete"))
                                .on_hover_text(tr(
                                    "Ends the program without asking it to save, then deletes again.",
                                ))
                                .clicked()
                            {
                                actions.push(retry(vec![l.pid]));
                                keep = false;
                            }
                        });
                    }
                } else if locks::is_access_denied(&item.error) {
                    ui.label(
                        RichText::new(tr(
                            "You don't have permission to delete this. Items in protected folders such as Program Files need administrator rights.",
                        ))
                        .weak(),
                    );
                }
                ui.separator();
            }
        });
    ui.add_space(4.0);
    let all_lockers: Vec<u32> = {
        let mut pids: Vec<u32> = d
            .items
            .iter()
            .flat_map(|i| i.lockers.iter().map(|l| l.pid))
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    };
    let denied = d
        .items
        .iter()
        .any(|i| i.lockers.is_empty() && locks::is_access_denied(&i.error));
    ui.horizontal_wrapped(|ui| {
        if all_lockers.len() > 1 && ui.button(tr("End all and delete")).clicked() {
            actions.push(retry(all_lockers.clone()));
            keep = false;
        }
        if denied
            && ui
                .button(tr("Delete as administrator"))
                .on_hover_text(tr(
                    "Deletes permanently (not to the Recycle Bin), after Windows asks for administrator rights.",
                ))
                .clicked()
        {
            actions.push(Action::DeleteAsAdmin(d.paths()));
            keep = false;
        }
        if ui.button(tr("Try again")).clicked() {
            actions.push(retry(Vec::new()));
            keep = false;
        }
        if ui.button(tr("📋 Copy details")).clicked() {
            ui.ctx().copy_text(d.details());
        }
        if ui.button(tr("Close")).clicked() {
            keep = false;
        }
    });
    keep
}
