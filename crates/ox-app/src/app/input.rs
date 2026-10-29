//! Keyboard handling: shortcut commands and keyboard navigation in the file list.

use super::*;

impl OxApp {
    /// Clipboard shortcuts are disabled in eframe so that Ctrl+V works even when
    /// the clipboard holds files instead of text. Text fields still need them,
    /// so translate the keys back into clipboard events while one has focus.
    pub(super) fn remap_text_clipboard_keys(&mut self, ctx: &Context) {
        if !ctx.text_edit_focused() {
            return;
        }
        let wants_paste = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::Key { key: Key::V, pressed: true, modifiers, .. } if modifiers.command_only())
            })
        });
        let paste_text = if wants_paste {
            if self.text_clipboard.is_none() {
                self.text_clipboard = arboard::Clipboard::new().ok();
            }
            self.text_clipboard.as_mut().and_then(|c| c.get_text().ok())
        } else {
            None
        };
        ctx.input_mut(|i| {
            for ev in &mut i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = ev
                {
                    if !modifiers.command_only() {
                        continue;
                    }
                    match key {
                        Key::C => *ev = egui::Event::Copy,
                        Key::X => *ev = egui::Event::Cut,
                        Key::V => {
                            if let Some(t) = &paste_text {
                                *ev = egui::Event::Paste(t.replace("\r\n", "\n"));
                            }
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    pub(super) fn handle_shortcuts(&mut self, ctx: &Context) {
        use crate::shortcuts::Command as C;
        use egui::PointerButton;
        let folder = self.current_folder();
        let selection = self.tabs.active().view.selected_paths();
        let shows_files = self.tabs.active().location().shows_files();
        let in_bin = self.tabs.active().location().is_bin();
        let read_only = self.tabs.active().location().is_read_only();
        let has_sel = !selection.is_empty();

        // Which commands make sense here; only their keys are taken.
        let available = |c: C| match c {
            C::SelectAll => shows_files,
            // The Recycle Bin purges on Delete; archives are read-only.
            C::Delete | C::DeletePermanently => shows_files && has_sel && !read_only,
            C::Copy | C::CopyPath => shows_files && has_sel && !in_bin,
            C::Cut | C::Rename | C::Properties => shows_files && has_sel && !in_bin && !read_only,
            C::ShellMenu => {
                shows_files && has_sel && !in_bin && !read_only && ox_core::shell::supported()
            }
            C::Paste | C::NewFolder => shows_files && folder.is_some(),
            C::OpenInEditor => {
                shows_files && !in_bin && !read_only && (has_sel || folder.is_some())
            }
            _ => true,
        };
        let (pressed, back, forward) = ctx.input_mut(|i| {
            (
                self.keymap.pressed(i, available),
                i.pointer.button_pressed(PointerButton::Extra1),
                i.pointer.button_pressed(PointerButton::Extra2),
            )
        });
        let mut actions = Vec::new();
        if back {
            actions.push(Action::Back);
        }
        if forward {
            actions.push(Action::Forward);
        }
        for c in pressed {
            match c {
                C::NewTab => actions.push(Action::Tab(TabAction::New)),
                C::CloseTab => actions.push(Action::Tab(TabAction::Close(self.tabs.active))),
                C::NextTab => actions.push(Action::Tab(TabAction::Cycle(1))),
                C::PrevTab => actions.push(Action::Tab(TabAction::Cycle(-1))),
                C::ToggleSplit => actions.push(Action::Tab(TabAction::ToggleSplit)),
                C::CompareFolders => actions.push(Action::Compare(super::CompareAction::Start)),
                C::SwitchPane => actions.push(Action::Tab(TabAction::SwitchPane)),
                C::Back => actions.push(Action::Back),
                C::Forward => actions.push(Action::Forward),
                C::Up => actions.push(Action::Up),
                C::Refresh => actions.push(Action::Refresh),
                C::EditAddress => {
                    self.ui.address_editing = true;
                    self.ui.focus_address = true;
                }
                C::Search => self.ui.focus_search = true,
                C::Palette => self.ui.palette = Some(Default::default()),
                C::TogglePreview => {
                    self.settings.browsing.show_preview = !self.settings.browsing.show_preview;
                    actions.push(Action::SettingsChanged);
                }
                C::Settings => actions.push(Action::OpenInNewTab(Location::Settings, true)),
                C::Help => actions.push(Action::OpenInNewTab(Location::Help, true)),
                C::SelectAll => actions.push(Action::SelectAll),
                C::Copy => actions.push(Action::CopyItems(selection.clone())),
                C::Cut => actions.push(Action::CutItems(selection.clone())),
                C::CopyPath => actions.push(Action::CopyPathText(selection.clone())),
                C::Paste => actions.extend(folder.clone().map(Action::Paste)),
                C::Undo => self.undo_redo(false),
                C::Redo => self.undo_redo(true),
                C::NewFolder => actions.extend(folder.clone().map(Action::NewFolder)),
                C::Delete | C::DeletePermanently if in_bin => {
                    actions.push(Action::PurgeFromBin(selection.clone()));
                }
                C::Delete => actions.push(Action::Delete(selection.clone(), false)),
                C::DeletePermanently => actions.push(Action::Delete(selection.clone(), true)),
                C::Rename => match selection.len() {
                    1 => actions.push(Action::Rename(selection[0].clone())),
                    _ => actions.push(Action::BulkRename(selection.clone())),
                },
                C::Properties => actions.push(Action::Properties(selection.clone())),
                C::ShellMenu => actions.push(Action::ShellMenu(selection.clone())),
                C::ToggleTerminal => actions.push(Action::ToggleTerminal),
                C::OpenInEditor => actions.push(Action::OpenInEditor(if has_sel {
                    selection.clone()
                } else {
                    folder.clone().into_iter().collect()
                })),
            }
        }
        self.actions.extend(actions);
        if shows_files {
            self.keyboard_navigation(ctx);
        }
    }

    /// Arrow keys, Home/End, Enter and type-to-select in the file view.
    pub(super) fn keyboard_navigation(&mut self, ctx: &Context) {
        let tab = self.tabs.active_mut();
        let n = tab.view.visible.len();
        if n == 0 {
            return;
        }
        let mut target: Option<usize> = None;
        let mut extend = false;
        let mut open = false;
        let mut typed = String::new();
        let grid_cols = ctx
            .data(|d| d.get_temp::<usize>(egui::Id::unique(("grid_columns", tab.id))))
            .unwrap_or(1)
            .max(1);
        let step = if tab.view_mode == ox_core::settings::ViewMode::Grid {
            grid_cols
        } else {
            1
        };
        ctx.input_mut(|i| {
            let cur = tab.view.cursor;
            extend = i.modifiers.shift;
            for (key, delta) in [
                (Key::ArrowDown, step as isize),
                (Key::ArrowUp, -(step as isize)),
                (Key::ArrowRight, if step > 1 { 1 } else { 0 }),
                (Key::ArrowLeft, if step > 1 { -1 } else { 0 }),
                (Key::PageDown, 20),
                (Key::PageUp, -20),
            ] {
                if delta != 0
                    && (i.consume_key(Modifiers::NONE, key) || i.consume_key(Modifiers::SHIFT, key))
                {
                    target = Some(match cur {
                        Some(c) => (c as isize + delta).clamp(0, n as isize - 1) as usize,
                        None => 0,
                    });
                }
            }
            if i.consume_key(Modifiers::NONE, Key::Home)
                || i.consume_key(Modifiers::SHIFT, Key::Home)
            {
                target = Some(0);
            }
            if i.consume_key(Modifiers::NONE, Key::End) || i.consume_key(Modifiers::SHIFT, Key::End)
            {
                target = Some(n - 1);
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                open = true;
            }
            if !i.modifiers.command && !i.modifiers.alt {
                for ev in &i.events {
                    if let egui::Event::Text(t) = ev {
                        typed.push_str(t);
                    }
                }
            }
        });

        if !typed.trim().is_empty() {
            if self.ui.type_ahead_at.elapsed() > Duration::from_millis(900) {
                self.ui.type_ahead.clear();
            }
            self.ui.type_ahead.push_str(&typed.to_lowercase());
            self.ui.type_ahead_at = Instant::now();
            let prefix = self.ui.type_ahead.clone();
            target = (0..n).find(|&i| {
                tab.view
                    .visible_entry(i)
                    .is_some_and(|e| e.name.to_lowercase().starts_with(&prefix))
            });
            extend = false;
        }

        if let Some(t) = target {
            if extend {
                if tab.view.anchor.is_none() {
                    tab.view.anchor = tab.view.cursor.or(Some(t));
                }
                tab.view.select_range(t, false);
            } else {
                tab.view.select_only(t);
            }
            tab.view.scroll_to = Some(t);
        }
        if open && !tab.location().is_bin() {
            let selected = tab.view.selected_paths();
            for p in selected {
                self.actions.push(Action::Open(p));
            }
        }
    }
}
