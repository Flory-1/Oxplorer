//! Remote locations: the dialogs (a place, a password, a host key, rename),
//! the address bar's crumbs and the context menus of remote folders.

use super::*;
use egui::{Context, Key};
use ox_core::remote::url::addr_from_parts;
use ox_core::remote::{AuthMethod, Protocol, RemoteAddr, RemotePlace};

use crate::app::{RemoteAction, RemoteDialog};

/// What a dialog's button does once the dialog is drawn.
type Then = Option<Box<dyn FnOnce(&mut OxApp)>>;

const WARN: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

/// The fields of the "remote location" dialog.
pub struct PlaceForm {
    /// The place being edited (`None`: a new one).
    index: Option<usize>,
    name: String,
    protocol: Protocol,
    host: String,
    port: String,
    user: String,
    path: String,
    /// 0 = agent and keys, then password; 1 = password; 2 = key file.
    auth: usize,
    key_file: String,
    password: String,
    remember: bool,
    error: Option<&'static str>,
    focus: bool,
}

impl PlaceForm {
    pub fn new(addr: Option<&RemoteAddr>) -> Self {
        let mut form = Self {
            index: None,
            name: String::new(),
            protocol: Protocol::Sftp,
            host: String::new(),
            port: String::new(),
            user: String::new(),
            path: String::new(),
            auth: 0,
            key_file: String::new(),
            password: String::new(),
            remember: ox_core::remote::credentials::supported(),
            error: None,
            focus: true,
        };
        if let Some(a) = addr {
            form.fill(a);
        }
        form
    }

    pub fn edit(index: usize, place: &RemotePlace) -> Self {
        let mut form = Self::new(Some(&place.addr));
        form.index = Some(index);
        form.name = place.name.clone();
        form.remember = place.remember_password;
        (form.auth, form.key_file) = match &place.auth {
            AuthMethod::Auto => (0, String::new()),
            AuthMethod::Password => (1, String::new()),
            AuthMethod::KeyFile(p) => (2, p.display().to_string()),
        };
        form
    }

    fn fill(&mut self, a: &RemoteAddr) {
        self.protocol = a.protocol;
        self.host = a.host.clone();
        self.port = if a.port == a.protocol.default_port() {
            String::new()
        } else {
            a.port.to_string()
        };
        self.user = a.user.clone();
        self.path = a.path.clone();
    }

    fn to_place(&self) -> Result<RemotePlace, &'static str> {
        let addr = addr_from_parts(
            self.protocol,
            &self.host,
            &self.port,
            &self.user,
            &self.path,
        )?;
        let auth = match self.auth {
            _ if addr.protocol != Protocol::Sftp => AuthMethod::Password,
            1 => AuthMethod::Password,
            2 if self.key_file.trim().is_empty() => return Err("Choose the key file."),
            2 => AuthMethod::KeyFile(self.key_file.trim().trim_matches('"').into()),
            _ => AuthMethod::Auto,
        };
        Ok(RemotePlace {
            name: self.name.trim().to_owned(),
            addr,
            auth,
            remember_password: self.remember,
        })
    }
}

/// Draw the open remote dialog, if any.
pub fn show(app: &mut OxApp, ctx: &Context) {
    let Some(mut dialog) = app.remote.dialog.take() else {
        return;
    };
    let mut keep = true;
    let mut done: Then = None;
    let modal = egui::Modal::new(Id::unique("remote_dialog")).show(ctx, |ui| {
        ui.set_width(440.0);
        match &mut dialog {
            RemoteDialog::Place(form) => place_dialog(ui, form, &mut keep, &mut done),
            RemoteDialog::Password {
                addr,
                text,
                remember,
                rejected,
                then,
                focus,
            } => {
                ui.heading(ox_core::trf!(
                    "Password for {server}",
                    server = addr.server_url()
                ));
                ui.add_space(6.0);
                if *rejected {
                    ui.colored_label(WARN, tr("The user name or password is wrong."));
                }
                if addr.protocol == Protocol::Sftp {
                    ui.label(RichText::new(tr("For a key file: its passphrase.")).weak());
                }
                let resp = ui.add(
                    egui::TextEdit::singleline(text)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );
                if std::mem::take(focus) {
                    resp.request_focus();
                }
                if ox_core::remote::credentials::supported() {
                    ui.checkbox(remember, tr("Remember the password (Windows Credential Manager)"));
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new(tr("Connect")).strong()).clicked() || enter {
                        let (addr, pw, remember, then) =
                            (addr.clone(), std::mem::take(text), *remember, *then);
                        done = Some(Box::new(move |app| {
                            app.remote_password(addr, &pw, remember, then)
                        }));
                        keep = false;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
            }
            RemoteDialog::HostKey {
                addr,
                question,
                then,
                confirmed,
            } => {
                let server = format!("{}:{}", question.host, question.port);
                if question.changed {
                    ui.heading(RichText::new(tr("⚠ The server's identity has changed")).color(WARN));
                    ui.add_space(6.0);
                    ui.label(ox_core::trf!(
                        "The key of {server} isn't the one saved before. Someone may be intercepting the connection, or the server was set up again. Only go on if its administrator confirms the new key.",
                        server = server
                    ));
                } else {
                    ui.heading(tr("Trust this server?"));
                    ui.add_space(6.0);
                    ui.label(ox_core::trf!(
                        "Oxplorer doesn't know {server} yet. Compare the key's fingerprint with the one its administrator gives you before you trust it.",
                        server = server
                    ));
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("{}\n{}", question.key_type, question.fingerprint))
                        .monospace(),
                );
                if question.changed {
                    ui.add_space(4.0);
                    ui.checkbox(
                        confirmed,
                        tr("I checked the new fingerprint with the server's administrator"),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = if question.changed {
                        tr("Replace the saved key and connect")
                    } else {
                        tr("Trust and connect")
                    };
                    let enabled = !question.changed || *confirmed;
                    if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                        let (addr, q, then) = (addr.clone(), question.clone(), *then);
                        done = Some(Box::new(move |app| app.remote_trust(addr, &q, then)));
                        keep = false;
                    }
                    if ui.button(RichText::new(tr("Cancel")).strong()).clicked() {
                        keep = false;
                    }
                });
            }
            RemoteDialog::Rename { path, text, focus } => {
                ui.heading(tr("Rename"));
                ui.add_space(6.0);
                let resp = ui.add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY));
                if std::mem::take(focus) {
                    resp.request_focus();
                }
                let name = text.trim().to_owned();
                let valid = !name.is_empty() && !name.contains(['/', '\\']) && name != "." && name != "..";
                if !valid && !name.is_empty() {
                    ui.colored_label(WARN, tr("A name can't contain / or \\."));
                }
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add_enabled(valid, egui::Button::new(tr("Rename"))).clicked()
                        || (enter && valid)
                    {
                        if name != display_name(path) {
                            let path = path.clone();
                            done = Some(Box::new(move |app| app.remote_rename(&path, &name)));
                        }
                        keep = false;
                    }
                    if ui.button(tr("Cancel")).clicked() {
                        keep = false;
                    }
                });
            }
        }
    });
    if modal.should_close() {
        keep = false;
    }
    if keep {
        app.remote.dialog = Some(dialog);
    }
    if let Some(f) = done {
        f(app);
    }
}

fn place_dialog(ui: &mut Ui, form: &mut PlaceForm, keep: &mut bool, done: &mut Then) {
    ui.heading(if form.index.is_some() {
        tr("Edit remote location")
    } else {
        tr("Add a remote location")
    });
    ui.add_space(6.0);
    egui::Grid::new("remote_place")
        .num_columns(2)
        .spacing([12.0, 8.0])
        .show(ui, |ui| {
            ui.label(tr("Protocol"));
            egui::ComboBox::from_id_salt("remote_protocol")
                .selected_text(form.protocol.label())
                .show_ui(ui, |ui| {
                    for p in Protocol::ALL {
                        ui.selectable_value(&mut form.protocol, p, p.label());
                    }
                });
            ui.end_row();

            ui.label(tr("Server"));
            ui.horizontal(|ui| {
                let host = ui.add(
                    egui::TextEdit::singleline(&mut form.host)
                        .hint_text(tr("name, IP address or https://…"))
                        .desired_width(220.0),
                );
                if std::mem::take(&mut form.focus) {
                    host.request_focus();
                }
                ui.label(tr("Port"));
                ui.add(
                    egui::TextEdit::singleline(&mut form.port)
                        .hint_text(form.protocol.default_port().to_string())
                        .desired_width(54.0),
                );
            });
            ui.end_row();

            ui.label(tr("User name"));
            ui.add(
                egui::TextEdit::singleline(&mut form.user)
                    .hint_text(if form.protocol == Protocol::Sftp {
                        tr("your Windows user name")
                    } else {
                        tr("empty: anonymous")
                    })
                    .desired_width(f32::INFINITY),
            );
            ui.end_row();

            ui.label(tr("Folder"));
            ui.add(
                egui::TextEdit::singleline(&mut form.path)
                    .hint_text(tr("/path (empty: where the login starts)"))
                    .desired_width(f32::INFINITY),
            );
            ui.end_row();

            if form.protocol == Protocol::Sftp {
                ui.label(tr("Log in with"));
                let choices = [
                    tr("SSH agent or keys, then password"),
                    tr("Password"),
                    tr("Key file"),
                ];
                egui::ComboBox::from_id_salt("remote_auth")
                    .selected_text(choices[form.auth.min(2)])
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for (i, c) in choices.iter().enumerate() {
                            ui.selectable_value(&mut form.auth, i, *c);
                        }
                    });
                ui.end_row();
                if form.auth == 2 {
                    ui.label(tr("Key file"));
                    ui.add(
                        egui::TextEdit::singleline(&mut form.key_file)
                            .hint_text(r"%USERPROFILE%\.ssh\id_ed25519")
                            .desired_width(f32::INFINITY),
                    );
                    ui.end_row();
                }
            }

            ui.label(tr("Password"));
            ui.add(
                egui::TextEdit::singleline(&mut form.password)
                    .password(true)
                    .hint_text(if form.index.is_some() {
                        tr("unchanged")
                    } else {
                        tr("asked when connecting")
                    })
                    .desired_width(f32::INFINITY),
            );
            ui.end_row();

            ui.label(tr("Name"));
            ui.add(
                egui::TextEdit::singleline(&mut form.name)
                    .hint_text(tr("shown in the sidebar (optional)"))
                    .desired_width(f32::INFINITY),
            );
            ui.end_row();
        });
    if ox_core::remote::credentials::supported() {
        ui.checkbox(
            &mut form.remember,
            tr("Remember the password (Windows Credential Manager)"),
        );
    }
    if !form.protocol.is_secure() {
        ui.label(
            RichText::new(tr(
                "⚠ Without encryption: the password and files can be read on the network.",
            ))
            .color(WARN),
        );
    }
    if let Some(e) = form.error {
        ui.colored_label(WARN, tr(e));
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let save = ui.button(tr("Save")).clicked();
        let connect = ui
            .button(RichText::new(tr("Save and connect")).strong())
            .clicked();
        if ui.button(tr("Cancel")).clicked() {
            *keep = false;
        }
        if save || connect {
            match form.to_place() {
                Ok(place) => {
                    let (index, password) = (form.index, std::mem::take(&mut form.password));
                    *done = Some(Box::new(move |app| {
                        let addr = place.addr.clone();
                        app.save_remote_place(index, place, &password);
                        if connect {
                            app.push(Action::Remote(RemoteAction::Open(addr, false)));
                        }
                    }));
                    *keep = false;
                }
                Err(e) => form.error = Some(e),
            }
        }
    });
}

/// The address bar of a remote folder: the server, then each folder.
pub fn crumbs(ui: &mut Ui, addr: &RemoteAddr, actions: &mut Vec<Action>) {
    let server = if addr.user.is_empty() {
        addr.host.clone()
    } else {
        format!("{}@{}", addr.user, addr.host)
    };
    let crumb = |ui: &mut Ui, text: String| ui.add(egui::Button::new(text).frame(false)).clicked();
    if crumb(ui, format!("🌐 {server}")) {
        actions.push(Action::Navigate(Location::Remote(addr.with_path("/"))));
    }
    let mut path = String::new();
    for part in addr.path.split('/').filter(|p| !p.is_empty()) {
        path = ox_core::remote::url::join_path(&path, part);
        ui.label(RichText::new(" › ").weak());
        if crumb(ui, part.to_owned()) {
            actions.push(Action::Navigate(Location::Remote(addr.with_path(&path))));
        }
    }
}

/// The context menu of remote items.
pub fn item_menu(ui: &mut Ui, t: &MenuTarget<'_>, actions: &mut Vec<Action>) {
    let sel = t.selection.to_vec();
    if ui.button(tr("📂 Open")).clicked() {
        for p in &sel {
            actions.push(Action::Open(p.clone()));
        }
    }
    if let Some(addr) = t.single_dir.and_then(RemoteAddr::from_path) {
        if ui.button(tr("📑 Open in new tab")).clicked() {
            actions.push(Action::OpenInNewTab(Location::Remote(addr), false));
        }
    }
    ui.separator();
    if ui
        .add(egui::Button::new(tr("✂ Cut")).shortcut_text(shortcuts::label(Command::Cut)))
        .clicked()
    {
        actions.push(Action::CutItems(sel.clone()));
    }
    if ui
        .add(egui::Button::new(tr("📋 Copy")).shortcut_text(shortcuts::label(Command::Copy)))
        .clicked()
    {
        actions.push(Action::CopyItems(sel.clone()));
    }
    if let Some(dir) = t.single_dir {
        if ui
            .add_enabled(t.can_paste, egui::Button::new(tr("📥 Paste into folder")))
            .clicked()
        {
            actions.push(Action::Paste(dir.to_path_buf()));
        }
    }
    let local: Vec<&(String, PathBuf)> = t
        .destinations
        .iter()
        .filter(|(_, p)| !ox_core::remote::is_remote_path(p))
        .collect();
    if !local.is_empty() {
        ui.menu_button(tr("📤 Download to"), |ui| {
            for (label, dest) in local {
                if ui.button(label).clicked() {
                    actions.push(Action::Transfer(
                        TransferKind::Copy,
                        sel.clone(),
                        dest.clone(),
                    ));
                }
            }
        });
    }
    ui.separator();
    if sel.len() == 1
        && ui
            .add(egui::Button::new(tr("✏ Rename")).shortcut_text(shortcuts::label(Command::Rename)))
            .clicked()
    {
        actions.push(Action::Rename(sel[0].clone()));
    }
    if ui
        .add(
            egui::Button::new(tr("⛔ Delete permanently"))
                .shortcut_text(shortcuts::label(Command::Delete)),
        )
        .clicked()
    {
        actions.push(Action::Delete(sel.clone(), true));
    }
    ui.separator();
    if ui
        .add(
            egui::Button::new(tr("🔗 Copy path"))
                .shortcut_text(shortcuts::label(Command::CopyPath)),
        )
        .clicked()
    {
        actions.push(Action::CopyPathText(sel.clone()));
    }
}

/// The context menu of a remote folder's empty space.
pub fn background_menu(ui: &mut Ui, folder: &Path, can_paste: bool, actions: &mut Vec<Action>) {
    if ui
        .add(
            egui::Button::new(tr("📁 New folder"))
                .shortcut_text(shortcuts::label(Command::NewFolder)),
        )
        .clicked()
    {
        actions.push(Action::NewFolder(folder.to_path_buf()));
    }
    if ui
        .add_enabled(
            can_paste,
            egui::Button::new(tr("📥 Paste")).shortcut_text(shortcuts::label(Command::Paste)),
        )
        .clicked()
    {
        actions.push(Action::Paste(folder.to_path_buf()));
    }
    if ui
        .add(egui::Button::new(tr("🔄 Refresh")).shortcut_text(shortcuts::label(Command::Refresh)))
        .clicked()
    {
        actions.push(Action::Refresh);
    }
    if ui
        .add(
            egui::Button::new(tr("☑ Select all"))
                .shortcut_text(shortcuts::label(Command::SelectAll)),
        )
        .clicked()
    {
        actions.push(Action::SelectAll);
    }
    ui.separator();
    let addr = RemoteAddr::from_path(folder);
    if ui.button(tr("⭐ Save as remote location")).clicked() {
        actions.push(Action::Remote(RemoteAction::Add(addr.clone())));
    }
    if let Some(addr) = addr {
        if ui.button(tr("🔗 Copy path")).clicked() {
            actions.push(Action::CopyPathText(vec![folder.to_path_buf()]));
        }
        if ui.button(tr("✖ Disconnect")).clicked() {
            actions.push(Action::Remote(RemoteAction::Disconnect(addr)));
        }
    }
}
