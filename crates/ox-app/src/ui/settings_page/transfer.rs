//! The Transfers page of the settings.

use egui::{RichText, Ui};
use ox_core::i18n::t as tr;
use ox_core::settings::Settings;
use ox_core::transfer::when_done::WhenDone;
use ox_core::transfer::ConflictPolicy;

use super::{row, toggle};

pub(super) fn transfer(ui: &mut Ui, s: &mut Settings) {
    let t = &mut s.transfer;
    ui.label(
        RichText::new(tr(
            "Oxplorer copies files with its own multi-threaded engine instead of the Windows copy dialog. \
             On Windows each file is copied with CopyFileExW (kernel fast path), large files bypass the cache.",
        ))
        .weak(),
    );
    ui.add_space(10.0);
    row(
        ui,
        tr("Parallel file copies"),
        tr("Auto: one file at a time on HDDs, 8 on SSDs. More helps with many small files on SSDs."),
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.workers, 0..=32).custom_formatter(|v, _| {
                    if v == 0.0 {
                        tr("Auto").into()
                    } else {
                        format!("{v:.0}")
                    }
                }),
            );
        },
    );
    row(
        ui,
        tr("Parallel jobs"),
        tr("How many copy jobs run at once; the rest wait in a queue."),
        |ui| {
            ui.add(egui::Slider::new(&mut t.max_concurrent_jobs, 1..=8));
        },
    );
    toggle(
        ui,
        &mut t.per_disk_queue,
        tr("One job per drive at a time"),
        tr("Copies on the same drive wait for each other; different drives still run in parallel."),
    );
    toggle(
        ui,
        &mut t.verify,
        tr("Verify copied files"),
        tr("Compare every copy with its original (xxh3). Slower, but moves only delete originals that were copied correctly."),
    );
    row(
        ui,
        tr("Speed limit"),
        tr("Per transfer. 0 = unlimited. Useful to keep the PC responsive during big copies."),
        |ui| {
            ui.add(
                egui::DragValue::new(&mut t.speed_limit_mib)
                    .range(0..=10_000)
                    .custom_formatter(|v, _| {
                        if v == 0.0 {
                            tr("Unlimited").into()
                        } else {
                            format!("{v:.0} MiB/s")
                        }
                    }),
            );
        },
    );
    row(
        ui,
        tr("Buffer size"),
        tr("Used by the portable copy loop."),
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.buffer_kib, 256..=65536)
                    .logarithmic(true)
                    .suffix(tr(" KiB")),
            );
        },
    );
    row(
        ui,
        tr("Unbuffered I/O from"),
        tr("Files at least this big skip the Windows file cache."),
        |ui| {
            ui.add(
                egui::Slider::new(&mut t.unbuffered_threshold_mib, 16..=8192)
                    .logarithmic(true)
                    .suffix(tr(" MiB")),
            );
        },
    );
    row(
        ui,
        tr("When a file already exists"),
        tr("\"Ask me\" asks once per job, not once per file."),
        |ui| {
            egui::ComboBox::from_id_salt("conflict")
                .selected_text(t.default_conflict.label())
                .show_ui(ui, |ui| {
                    for p in ConflictPolicy::ALL {
                        ui.selectable_value(&mut t.default_conflict, p, p.label());
                    }
                });
        },
    );
    toggle(
        ui,
        &mut t.ask_on_error,
        tr("Ask when a file fails"),
        tr("The job waits: retry, skip, or skip all errors of the job. Off: errors are listed when the job ends."),
    );
    row(
        ui,
        tr("When all transfers are done"),
        tr("Sleep waits 30 seconds, which you can cancel, and never follows a cancelled transfer."),
        |ui| {
            egui::ComboBox::from_id_salt("when_done")
                .selected_text(t.when_done.label())
                .show_ui(ui, |ui| {
                    for w in WhenDone::ALL {
                        ui.selectable_value(&mut t.when_done, w, w.label());
                    }
                });
        },
    );
    toggle(
        ui,
        &mut t.preserve_timestamps,
        tr("Keep modification dates"),
        "",
    );
    toggle(
        ui,
        &mut t.use_system_clipboard,
        tr("Share clipboard with Windows Explorer"),
        tr("Copy in Explorer, paste in Oxplorer (and the other way around)."),
    );
    toggle(
        ui,
        &mut t.drag_moves_on_same_drive,
        tr("Dragging moves on the same drive"),
        tr("Like Explorer. Hold Ctrl to copy, Shift to move."),
    );
    row(
        ui,
        tr("Transfer history"),
        tr("Every finished copy and move, with errors."),
        |ui| {
            let log = ox_core::transfer::history::log_path();
            if ui
                .add_enabled(log.exists(), egui::Button::new(tr("📜 Open log")))
                .clicked()
            {
                let _ = ox_core::fsops::open_default(&log);
            }
        },
    );
}
