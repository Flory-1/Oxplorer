//! Flat view on and off, and going to the folder an item is in.

use super::*;

impl OxApp {
    pub(super) fn toggle_flat_view(&mut self) {
        let tab = self.tabs.active_mut();
        if !tab.can_flat() {
            self.toast(tr("Flat view works in folders only."));
            return;
        }
        let on = !tab.is_flat();
        tab.set_flat(on);
    }

    /// Navigate to the folder `path` is in and select it there (with the
    /// flat view off, so it's seen in its own folder).
    pub(super) fn show_in_folder(&mut self, path: PathBuf) {
        let Some(parent) = path.parent().map(Path::to_path_buf) else {
            return;
        };
        // An item found inside an archive is shown in the archive view.
        let loc = match archive::split_virtual(&path) {
            Some((archive, inner)) if !inner.as_os_str().is_empty() => Location::Archive {
                archive,
                inner: inner.parent().map(Path::to_path_buf).unwrap_or_default(),
            },
            _ => Location::Folder(parent),
        };
        self.navigate(loc.clone());
        let tab = self.tabs.active_mut();
        if *tab.location() == loc {
            tab.set_flat(false);
            tab.view.select_after_load = Some(path);
        }
    }
}
