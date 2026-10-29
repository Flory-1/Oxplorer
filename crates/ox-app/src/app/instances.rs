//! More windows: Ctrl+N, moving a tab into a window of its own, and what
//! several running windows share (the saved session, settings, favorites,
//! tags; see `ox_core::instance` and `ox_core::shared`).

use super::*;
use crate::tabs::SessionTab;
use ox_core::instance::{self, Launch, SessionLock};
use ox_core::places::QuickAccess;
use ox_core::shared::Shared;
use ox_core::tags::TagFile;

/// How this window was started (from the command line, see `main.rs`).
#[derive(Default)]
pub struct Start {
    /// A folder to open in a new tab.
    pub open: Option<PathBuf>,
    /// Tabs handed over by the window that opened this one.
    pub tabs: Option<Session>,
    /// Held if this window owns the saved session.
    pub session_lock: Option<SessionLock>,
}

/// This window among the others.
pub struct Instance {
    pub settings: Shared<Settings>,
    pub quick: Shared<QuickAccess>,
    pub tags: Shared<TagFile>,
    checked: Instant,
    session_lock: Option<SessionLock>,
    /// Where a dragged tab last was (the pointer may be gone outside the window).
    drag_pos: Option<egui::Pos2>,
}

/// How often to look whether another window changed the shared files.
const CHECK_EVERY: Duration = Duration::from_secs(2);

impl Instance {
    pub fn new(session_lock: Option<SessionLock>) -> Self {
        Self {
            settings: Shared::new(Settings::path()),
            quick: Shared::new(QuickAccess::path()),
            tags: Shared::new(TagStore::path()),
            checked: Instant::now(),
            session_lock,
            drag_pos: None,
        }
    }

    /// This window restores the saved session and saves it when it closes.
    pub fn owns_session(&self) -> bool {
        self.session_lock.is_some()
    }
}

/// A tab dragged this far below the tab strip opens in a new window.
const TEAR_OFF: f32 = 48.0;

impl OxApp {
    /// Ctrl+N: a new window showing what this tab shows.
    pub(super) fn new_window(&mut self) {
        let loc = self.tabs.active().location().clone();
        self.open_window(&loc, false, None);
    }

    /// Open a tab's place in a new window and close it here (unless it's
    /// the only one besides the Home tab: then this window would be empty).
    pub(super) fn move_to_new_window(&mut self, id: TabId, pos: Option<[f32; 2]>) {
        if self.tabs.is_home(id) {
            return;
        }
        let Some(tab) = self.tabs.get(id) else {
            return;
        };
        let (loc, pinned) = (tab.location().clone(), tab.pinned);
        if self.open_window(&loc, pinned, pos) && self.tabs.tabs.len() > 2 {
            self.tabs.close(id);
        }
    }

    /// Start a new window showing `loc`: a folder goes on the command line,
    /// anything else (a search, Settings, a pinned tab…) in a file.
    fn open_window(&mut self, loc: &Location, pinned: bool, pos: Option<[f32; 2]>) -> bool {
        let mut launch = Launch {
            new_window: true,
            pos,
            ..Default::default()
        };
        match loc {
            Location::Home => {}
            Location::Folder(p) if !pinned => launch.folder = Some(p.display().to_string()),
            other => {
                let session = Session {
                    tabs: vec![
                        SessionTab {
                            location: Location::Home,
                            group: None,
                            pinned: false,
                        },
                        SessionTab {
                            location: other.clone(),
                            group: None,
                            pinned,
                        },
                    ],
                    groups: Vec::new(),
                    active: 1,
                };
                let path = instance::handoff_path();
                if let Err(e) = ox_core::settings::save_toml(&path, &session) {
                    self.toast(ox_core::trf!("Couldn't open a new window: {e}", e = e));
                    return false;
                }
                launch.tabs = Some(path);
            }
        }
        match instance::open_window(&launch) {
            Ok(()) => true,
            Err(e) => {
                if let Some(p) = &launch.tabs {
                    let _ = std::fs::remove_file(p);
                }
                self.toast(ox_core::trf!("Couldn't open a new window: {e}", e = e));
                false
            }
        }
    }

    /// A tab dropped outside the window, or well below the tab strip, goes
    /// into a new window there. While it's on its way, say so.
    pub(super) fn drag_tab_out(&mut self, ctx: &Context) {
        let Some(drag) = egui::DragAndDrop::payload::<crate::ui::DragTab>(ctx) else {
            self.instance.drag_pos = None;
            return;
        };
        // No position: the pointer left the window.
        let latest = ctx.input(|i| i.pointer.latest_pos());
        let Some(pos) = latest.or(self.instance.drag_pos) else {
            return;
        };
        self.instance.drag_pos = Some(pos);
        let window = ctx.content_rect();
        let strip: Option<egui::Rect> =
            ctx.data(|d| d.get_temp(crate::ui::header::tab_strip_rect_id()));
        let below = strip.is_some_and(|s| pos.y > s.bottom() + TEAR_OFF);
        let inside = latest.is_some() && window.contains(pos);
        if !below && inside {
            return;
        }
        if !ctx.input(|i| i.pointer.any_released()) {
            if inside {
                egui::Area::new(egui::Id::unique("tear_off_hint"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(pos + egui::vec2(14.0, 18.0))
                    .interactable(false)
                    .show(ctx, |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(tr("🗔 Release to open in a new window"));
                        });
                    });
            }
            return;
        }
        egui::DragAndDrop::clear_payload(ctx);
        // Where the pointer is on the screen, in the points windows are placed in.
        let screen = ctx
            .input(|i| i.viewport().inner_rect)
            .map(|r| (r.min + pos.to_vec2()) * ctx.zoom_factor())
            // Put the new window's tabs under the pointer.
            .map(|p| [p.x - 120.0, p.y - 16.0]);
        self.move_to_new_window(drag.0, screen);
    }

    /// Pick up what other windows changed in the shared files (every few
    /// seconds, and only when a file changed).
    pub(super) fn poll_shared_files(&mut self) {
        if self.instance.checked.elapsed() < CHECK_EVERY {
            return;
        }
        self.instance.checked = Instant::now();
        if let Some(s) = self.instance.settings.poll(&self.settings) {
            self.settings = s;
            self.tabs.mark_stale(None);
        }
        if let Some(q) = self.instance.quick.poll(&self.quick) {
            self.quick = q;
        }
        if let Some(t) = self.instance.tags.poll(&self.tags.to_file()) {
            self.adopt_tags(t);
        }
    }

    fn adopt_tags(&mut self, file: TagFile) {
        let version = self.tags.version + 1;
        self.tags = TagStore::from_file(file);
        self.tags.version = version;
        for t in &mut self.tabs.tabs {
            if matches!(t.location(), Location::Tagged(_)) {
                t.stale = true;
            }
        }
    }

    /// Save the settings, keeping what another window changed meanwhile.
    pub(super) fn save_settings(&mut self) -> std::io::Result<()> {
        if let Some(merged) = self.instance.settings.save(&self.settings)? {
            self.settings = merged;
        }
        Ok(())
    }

    pub fn save_quick(&mut self) {
        match self.instance.quick.save(&self.quick) {
            Ok(Some(merged)) => self.quick = merged,
            Ok(None) => {}
            Err(e) => log::warn!("could not save quick access: {e}"),
        }
    }

    pub(super) fn save_tags(&mut self) -> std::io::Result<()> {
        if let Some(merged) = self.instance.tags.save(&self.tags.to_file())? {
            self.adopt_tags(merged);
        }
        Ok(())
    }

    /// Only the window that owns the saved session writes it.
    pub(super) fn save_session(&self) {
        if !self.instance.owns_session() {
            return;
        }
        if let Err(e) = self.tabs.to_session().save() {
            log::warn!("could not save session: {e}");
        }
    }
}
