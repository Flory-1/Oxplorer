//! Settings → Search index: the optional Oxplorer index service (Windows).

use egui::{RichText, Ui};
use ox_core::i18n::t as tr;
use ox_core::index::helper::{self, Status};

use super::{row, toggle};
use crate::app::{Action, OxApp};

/// The toggle and how the service is.
pub(super) fn section(ui: &mut Ui, app: &mut OxApp) {
    if !cfg!(windows) {
        return;
    }
    let se = &mut app.settings.search;
    let was = se.index_service;
    toggle(
        ui,
        &mut se.index_service,
        tr("Use the Oxplorer index service"),
        tr(
            "Reads NTFS drives from the master file table and follows the change journal \
             without running Oxplorer as administrator. The service is installed by the \
             installer (task \"Fast index service\", needs admin rights). It shows you the \
             names of all files on the drives, also in folders you can't open.",
        ),
    );
    if se.index_service != was {
        helper::set_enabled(se.index_service);
        if se.index_service {
            // Read the drives through the service right away.
            app.actions.push(Action::RebuildIndex);
        }
    }
    if !se.index_service {
        return;
    }
    let text = match helper::status() {
        None => tr("Checking…").to_owned(),
        Some(Status::Running(drives)) => {
            let drives: Vec<String> = drives.iter().map(|d| format!("{d}:")).collect();
            ox_core::trf!("Running · reads {v1}", v1 = drives.join(", "))
        }
        Some(Status::Stopped) => tr("Installed · starts when needed").to_owned(),
        Some(Status::NoAccess) => tr("Installed for another user").to_owned(),
        Some(Status::Unavailable) => tr("Installed, but it doesn't answer").to_owned(),
        Some(Status::NotInstalled | Status::Unsupported) => {
            tr("Not installed: run the installer for all users and choose \"Fast index service\"")
                .to_owned()
        }
    };
    row(ui, tr("Index service"), "", |ui| {
        ui.label(RichText::new(text).weak());
    });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_secs(2));
}
