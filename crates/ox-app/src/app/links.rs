//! Making links (from the New menu's dialog, or by dragging with
//! Ctrl+Shift) and going to where a link points.

use ox_core::links::{self, LinkKind};
use ox_core::undo::{Kind, Link, Op};

use super::*;

impl OxApp {
    /// Open the dialog for a new link in `dir`, pointing at the selected
    /// item to begin with.
    pub(super) fn new_link_dialog(&mut self, dir: PathBuf, kind: LinkKind) {
        let selection = self.tabs.active().view.selected_paths();
        let target = match selection.as_slice() {
            [one] => Some(one.as_path()),
            _ => None,
        };
        let d = crate::ui::link_dialog::LinkDialog::new(dir, kind, target);
        self.dialog = Some(Dialog::NewLink(Box::new(d)));
    }

    /// Make a link (Ctrl+Z removes it again) and select it.
    pub fn create_link(
        &mut self,
        kind: LinkKind,
        link: &Path,
        target: &Path,
    ) -> Result<(), String> {
        links::create(kind, link, target)?;
        self.linked(vec![Link {
            path: link.to_path_buf(),
            target: target.to_path_buf(),
            kind,
        }]);
        self.toast(ox_core::trf!(
            "{kind} \"{name}\" created",
            kind = kind.label(),
            name = entry::display_name(link)
        ));
        Ok(())
    }

    /// Record new links for undo, show them and select the first (when
    /// they're in the folder on screen).
    fn linked(&mut self, made: Vec<Link>) {
        let Some(first) = made.first() else { return };
        let dir = first.path.parent().map(Path::to_path_buf);
        let tab = self.tabs.active_mut();
        if tab.location().folder() == dir.as_deref() {
            tab.view.select_after_load = Some(first.path.clone());
        }
        self.undo.record(Kind::NewLink, Op::Linked(made));
        self.mark_changed(dir.as_deref());
    }

    /// Items dragged with Ctrl+Shift onto `dest`: link to each of them from
    /// there (folders with a junction, files with a symbolic link or, where
    /// Windows doesn't allow those, a hard link).
    pub(super) fn link_dropped(&mut self, sources: Vec<PathBuf>, dest: PathBuf) {
        let mut made = Vec::new();
        let mut errors = Vec::new();
        let mut fallback = false;
        for src in &sources {
            match links::link_into(src, &dest) {
                Ok(m) => {
                    fallback |= m.fallback;
                    made.push((
                        m.kind,
                        Link {
                            path: m.link,
                            target: src.clone(),
                            kind: m.kind,
                        },
                    ));
                }
                Err(e) => errors.push(e),
            }
        }
        let text = match (made.as_slice(), errors.as_slice()) {
            ([], [e]) => e.clone(),
            ([], _) => ox_core::trf!(
                "No links created: {e}",
                e = errors.first().cloned().unwrap_or_default()
            ),
            ([(kind, l)], []) => ox_core::trf!(
                "{kind} \"{name}\" created",
                kind = kind.label(),
                name = entry::display_name(&l.path)
            ),
            (_, []) => ox_core::trf!(
                "{v1} created",
                v1 = ox_core::i18n::count(made.len() as u64, "link")
            ),
            (_, [e, ..]) => ox_core::trf!(
                "{v1} created, {v2} failed: {e}",
                v1 = ox_core::i18n::count(made.len() as u64, "link"),
                v2 = errors.len(),
                e = e
            ),
        };
        let text = if fallback {
            ox_core::trf!(
                "{text} · Hard links instead of symbolic links: Windows allows those only with Developer Mode on or for administrators.",
                text = text
            )
        } else {
            text
        };
        self.toast(text);
        self.linked(made.into_iter().map(|(_, l)| l).collect());
    }

    /// Go to the folder a link's target is in and select the target there.
    pub(super) fn show_link_target(&mut self, link: PathBuf) {
        let Some(info) = links::info(&link) else {
            self.toast(tr("This isn't a link."));
            return;
        };
        let target = info.resolved(&link);
        if !ox_core::fsops::exists(&target) {
            self.toast(ox_core::trf!(
                "The link points to \"{v1}\", which doesn't exist.",
                v1 = target.display()
            ));
            return;
        }
        if target.parent().is_none() {
            // A drive: there's nothing to select it in.
            self.navigate(Location::Folder(target));
        } else {
            self.show_in_folder(target);
        }
    }
}
