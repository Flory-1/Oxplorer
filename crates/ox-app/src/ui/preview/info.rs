//! The Info tab: what an item is, where, how big, its dates and attributes,
//! the details of its type, and checksums on request.

use egui::{RichText, Ui};
use ox_core::details::Details;
use ox_core::entry::FileEntry;
use ox_core::format;
use ox_core::i18n::t as tr;

use super::Checksums;
use crate::app::Action;

/// Name, size, place, dates and attributes, then the type's own details.
pub(super) fn info_grid(
    ui: &mut Ui,
    e: &FileEntry,
    details: Option<&Details>,
    facts: Vec<(&'static str, String)>,
    date_fmt: &str,
) {
    let mut rows = vec![(tr("Name"), e.name.clone())];
    rows.extend(info_rows(e, details, date_fmt));
    let attrs: Vec<String> = [
        (e.hidden, tr("hidden")),
        (e.readonly, tr("read-only")),
        (e.is_symlink, tr("link")),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, a)| a.to_owned())
    .chain(details.into_iter().flat_map(other_attributes))
    .collect();
    if !attrs.is_empty() {
        rows.push((tr("Attributes"), attrs.join(", ")));
    }
    rows.extend(facts);
    rows.extend(details.into_iter().flat_map(|d| d.facts.iter().cloned()));
    egui::Grid::new("preview_info")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 4.0))
        .show(ui, |ui| {
            for (name, value) in rows {
                ui.label(RichText::new(name).weak());
                ui.add(egui::Label::new(value).wrap());
                ui.end_row();
            }
        });
}

/// Type, size, place and dates of an item (the Info tab and the Properties
/// dialog). `details` adds what takes longer to find out.
pub fn info_rows(
    e: &FileEntry,
    details: Option<&Details>,
    date_fmt: &str,
) -> Vec<(&'static str, String)> {
    let size = |bytes: u64| {
        ox_core::trf!(
            "{v1} ({v2} bytes)",
            v1 = format::size(bytes),
            v2 = group_digits(bytes)
        )
    };
    let mut rows = vec![(tr("Type"), e.type_label())];
    if let Some(app) = details.and_then(|d| d.opens_with.clone()) {
        rows.push((tr("Opens with"), app));
    }
    if !e.is_dir {
        rows.push((tr("Size"), size(e.size)));
    }
    if let Some(bytes) = details.and_then(|d| d.size_on_disk) {
        rows.push((tr("Size on disk"), size(bytes)));
    }
    if let Some(parent) = e.path.parent() {
        rows.push((tr("Location"), parent.display().to_string()));
    }
    if let Some(target) = details.and_then(|d| d.link_target.as_ref()) {
        rows.push((tr("Link to"), target.display().to_string()));
    }
    // OneDrive & co.: the fresh status with the provider, else the listing's.
    let cloud = details.and_then(|d| d.cloud.as_ref());
    if let Some((status, provider)) = cloud {
        rows.push((tr("Status"), format!("{} ({provider})", status.label())));
    } else if let Some(status) = e.cloud {
        rows.push((tr("Status"), status.label().to_owned()));
    }
    let dates = [
        (tr("Modified"), e.modified),
        (tr("Created"), e.created),
        (tr("Accessed"), details.and_then(|d| d.accessed)),
        (tr("Date taken"), details.and_then(|d| d.date_taken)),
    ];
    for (label, t) in dates {
        if let Some(t) = t {
            rows.push((label, format::time(t, date_fmt)));
        }
    }
    rows
}

/// Attributes besides hidden, read-only and link, in lower case.
pub fn other_attributes(d: &Details) -> impl Iterator<Item = String> + '_ {
    d.attributes.iter().map(|a| a.to_lowercase())
}

/// 1234567 → "1,234,567".
pub(super) fn group_digits(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// SHA-256 and MD5 of the file, computed when asked for (big files take a
/// while), with buttons to copy them.
pub(super) fn checksums(
    ui: &mut Ui,
    state: &mut Option<Checksums>,
    e: &FileEntry,
    actions: &mut Vec<Action>,
) {
    if let Some(Checksums::Running(rx)) = state {
        if let Ok(result) = rx.try_recv() {
            *state = Some(Checksums::Done(result));
        }
    }
    match state {
        None => {
            if ui
                .button(tr("🔐 Compute checksums"))
                .on_hover_text(tr("SHA-256 and MD5 of the file"))
                .clicked()
            {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let path = e.path.clone();
                let ctx = ui.ctx().clone();
                std::thread::spawn(move || {
                    let result = ox_core::textinfo::checksums(&path).map_err(|e| e.to_string());
                    let _ = tx.send(result);
                    ctx.request_repaint();
                });
                *state = Some(Checksums::Running(rx));
            }
        }
        Some(Checksums::Running(_)) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(tr("Computing checksums…")).weak());
            });
        }
        Some(Checksums::Done(Err(err))) => {
            ui.colored_label(egui::Color32::from_rgb(0xe5, 0x48, 0x4d), err.as_str());
        }
        Some(Checksums::Done(Ok((sha, md5)))) => {
            for (name, value) in [("SHA-256", sha), ("MD5", md5)] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).weak());
                    if ui.small_button("📋").on_hover_text(tr("Copy")).clicked() {
                        ui.ctx().copy_text(value.clone());
                        actions.push(Action::Toast(ox_core::trf!("Copied {what}", what = name)));
                    }
                });
                ui.add(
                    egui::Label::new(RichText::new(value.as_str()).monospace().size(10.5))
                        .wrap_mode(egui::TextWrapMode::Wrap),
                );
            }
        }
    }
}
