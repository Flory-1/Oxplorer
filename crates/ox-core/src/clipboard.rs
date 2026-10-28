//! File clipboard: an internal copy/cut buffer that is mirrored to the system
//! clipboard on Windows (`CF_HDROP` + `Preferred DropEffect`), so files can be
//! copied between Oxplorer and Windows Explorer in both directions — but the
//! paste itself always runs through Oxplorer's own transfer engine.

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipMode {
    Copy,
    Cut,
}

#[derive(Debug, Clone, Default)]
pub struct FileClipboard {
    items: Vec<PathBuf>,
    mode: Option<ClipMode>,
    /// System clipboard sequence number right after we wrote to it.
    #[allow(dead_code)]
    system_seq: Option<u32>,
}

impl FileClipboard {
    /// Put files on the clipboard. If `system` is set they're also shared with other apps.
    pub fn set(&mut self, items: Vec<PathBuf>, mode: ClipMode, system: bool) {
        self.system_seq = None;
        if system {
            if let Err(e) = imp::write(&items, mode) {
                eprintln!("could not write system clipboard: {e}");
            }
            self.system_seq = imp::sequence();
        }
        self.items = items;
        self.mode = Some(mode);
    }

    /// Current clipboard content. If another app put files on the system
    /// clipboard since our last copy, those win (like in Explorer).
    pub fn get(&self, system: bool) -> Option<(Vec<PathBuf>, ClipMode)> {
        if system {
            let changed_elsewhere = imp::sequence() != self.system_seq || self.mode.is_none();
            if changed_elsewhere {
                if let Some(found) = imp::read() {
                    return Some(found);
                }
            }
        }
        let mode = self.mode?;
        (!self.items.is_empty()).then(|| (self.items.clone(), mode))
    }

    /// Is `path` currently cut (shown dimmed in the file view)?
    pub fn is_cut(&self, path: &std::path::Path) -> bool {
        self.mode == Some(ClipMode::Cut) && self.items.iter().any(|p| p == path)
    }

    /// After a cut + paste the clipboard is emptied, like Explorer does.
    pub fn clear(&mut self, system: bool) {
        self.items.clear();
        self.mode = None;
        if system {
            imp::clear();
            self.system_seq = imp::sequence();
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::ClipMode;
    use clipboard_win::{options, raw, Clipboard};
    use std::path::PathBuf;

    const DROPEFFECT_COPY: u32 = 1;
    const DROPEFFECT_MOVE: u32 = 2;

    fn drop_effect_format() -> Option<u32> {
        raw::register_format("Preferred DropEffect").map(|f| f.get())
    }

    pub fn write(items: &[PathBuf], mode: ClipMode) -> Result<(), String> {
        let _clip = Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
        let paths: Vec<String> = items
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        raw::set_file_list_with(&paths, options::DoClear).map_err(|e| e.to_string())?;
        if let Some(fmt) = drop_effect_format() {
            let effect = match mode {
                ClipMode::Copy => DROPEFFECT_COPY,
                ClipMode::Cut => DROPEFFECT_MOVE,
            };
            raw::set_without_clear(fmt, &effect.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn read() -> Option<(Vec<PathBuf>, ClipMode)> {
        let _clip = Clipboard::new_attempts(10).ok()?;
        let mut files = Vec::new();
        raw::get_file_list_path(&mut files).ok()?;
        if files.is_empty() {
            return None;
        }
        let mut mode = ClipMode::Copy;
        if let Some(fmt) = drop_effect_format() {
            let mut buf = Vec::new();
            if raw::get_vec(fmt, &mut buf).is_ok() && buf.len() >= 4 {
                let effect = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
                if effect & DROPEFFECT_MOVE != 0 {
                    mode = ClipMode::Cut;
                }
            }
        }
        Some((files, mode))
    }

    pub fn clear() {
        if let Ok(_clip) = Clipboard::new_attempts(10) {
            let _ = raw::empty();
        }
    }

    pub fn sequence() -> Option<u32> {
        raw::seq_num().map(|n| n.get())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ClipMode;
    use std::path::PathBuf;

    pub fn write(_items: &[PathBuf], _mode: ClipMode) -> Result<(), String> {
        Ok(())
    }

    pub fn read() -> Option<(Vec<PathBuf>, ClipMode)> {
        None
    }

    pub fn clear() {}

    pub fn sequence() -> Option<u32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_clipboard() {
        let mut c = FileClipboard::default();
        assert!(c.get(false).is_none());
        c.set(vec!["/a".into()], ClipMode::Cut, false);
        assert!(c.is_cut(std::path::Path::new("/a")));
        let (items, mode) = c.get(false).unwrap();
        assert_eq!(items, vec![PathBuf::from("/a")]);
        assert_eq!(mode, ClipMode::Cut);
        c.clear(false);
        assert!(c.get(false).is_none());
    }
}
