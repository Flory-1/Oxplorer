//! Colour tags for files and folders (like macOS Finder tags).
//!
//! Tags are stored by path in `tags.toml`. Each colour can be given a name
//! ("Red" → "Urgent"). Tags follow renames and moves made inside Oxplorer;
//! tags of items that no longer exist are dropped when the store loads.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::{load_toml, save_toml};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagColor {
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Grey,
}

impl TagColor {
    pub const ALL: [TagColor; 7] = [
        TagColor::Red,
        TagColor::Orange,
        TagColor::Yellow,
        TagColor::Green,
        TagColor::Blue,
        TagColor::Purple,
        TagColor::Grey,
    ];

    /// Default name, also used in `tag:` searches.
    pub fn name(self) -> &'static str {
        match self {
            TagColor::Red => "Red",
            TagColor::Orange => "Orange",
            TagColor::Yellow => "Yellow",
            TagColor::Green => "Green",
            TagColor::Blue => "Blue",
            TagColor::Purple => "Purple",
            TagColor::Grey => "Grey",
        }
    }

    pub fn rgb(self) -> [u8; 3] {
        match self {
            TagColor::Red => [0xe5, 0x48, 0x4d],
            TagColor::Orange => [0xf0, 0x8a, 0x24],
            TagColor::Yellow => [0xf0, 0xc4, 0x29],
            TagColor::Green => [0x3b, 0xb2, 0x73],
            TagColor::Blue => [0x4c, 0x8d, 0xf6],
            TagColor::Purple => [0x9b, 0x6b, 0xf2],
            TagColor::Grey => [0x8a, 0x8f, 0x98],
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&c| c == self).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TaggedItem {
    path: PathBuf,
    tags: Vec<TagColor>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct TagFile {
    /// Custom names, indexed like [`TagColor::ALL`] (empty = default name).
    labels: Vec<String>,
    items: Vec<TaggedItem>,
}

#[derive(Debug, Clone, Default)]
pub struct TagStore {
    labels: [String; 7],
    items: HashMap<PathBuf, Vec<TagColor>>,
    /// Bumped on every change, so views know when to redraw tag listings.
    pub version: u64,
}

impl TagStore {
    pub fn path() -> PathBuf {
        crate::config_dir().join("tags.toml")
    }

    pub fn load() -> Self {
        let file: TagFile = load_toml(&Self::path()).unwrap_or_default();
        let mut store = Self::from_file(file);
        store.prune();
        store
    }

    fn from_file(file: TagFile) -> Self {
        let mut labels: [String; 7] = Default::default();
        for (slot, label) in labels.iter_mut().zip(file.labels) {
            *slot = label;
        }
        let items = file
            .items
            .into_iter()
            .filter(|i| !i.tags.is_empty())
            .map(|i| (i.path, i.tags))
            .collect();
        Self {
            labels,
            items,
            version: 0,
        }
    }

    fn to_file(&self) -> TagFile {
        let mut items: Vec<TaggedItem> = self
            .items
            .iter()
            .map(|(path, tags)| TaggedItem {
                path: path.clone(),
                tags: tags.clone(),
            })
            .collect();
        items.sort_by(|a, b| a.path.cmp(&b.path));
        TagFile {
            labels: self.labels.to_vec(),
            items,
        }
    }

    pub fn save(&self) -> io::Result<()> {
        save_toml(&Self::path(), &self.to_file())
    }

    /// Forget items that no longer exist.
    pub fn prune(&mut self) {
        let before = self.items.len();
        self.items.retain(|p, _| p.symlink_metadata().is_ok());
        if self.items.len() != before {
            self.version += 1;
        }
    }

    /// Display name of a colour (custom label or the colour's name).
    pub fn label(&self, color: TagColor) -> &str {
        let custom = &self.labels[color.index()];
        if custom.trim().is_empty() {
            color.name()
        } else {
            custom
        }
    }

    /// The custom name of a colour ("" when it uses its default name).
    pub fn custom_label(&self, color: TagColor) -> &str {
        &self.labels[color.index()]
    }

    pub fn set_label(&mut self, color: TagColor, label: &str) {
        self.labels[color.index()] = label.trim().to_owned();
        self.version += 1;
    }

    /// Tags of `path`, in colour order.
    pub fn tags(&self, path: &Path) -> &[TagColor] {
        self.items.get(path).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn has(&self, path: &Path, color: TagColor) -> bool {
        self.tags(path).contains(&color)
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Add `color` to all `paths`, or remove it if every one already has it.
    pub fn toggle(&mut self, paths: &[PathBuf], color: TagColor) {
        let all = !paths.is_empty() && paths.iter().all(|p| self.has(p, color));
        for p in paths {
            let tags = self.items.entry(p.clone()).or_default();
            if all {
                tags.retain(|&c| c != color);
            } else if !tags.contains(&color) {
                tags.push(color);
                tags.sort();
            }
            if tags.is_empty() {
                self.items.remove(p);
            }
        }
        self.version += 1;
    }

    pub fn clear(&mut self, paths: &[PathBuf]) {
        for p in paths {
            self.items.remove(p);
        }
        self.version += 1;
    }

    /// `from` was renamed or moved to `to`: its tags (and those of
    /// everything inside it) go along.
    pub fn moved(&mut self, from: &Path, to: &Path) {
        let affected: Vec<PathBuf> = self
            .items
            .keys()
            .filter(|p| p.starts_with(from))
            .cloned()
            .collect();
        for old in affected {
            let Some(tags) = self.items.remove(&old) else {
                continue;
            };
            let new = match old.strip_prefix(from) {
                Ok(rest) if rest.as_os_str().is_empty() => to.to_path_buf(),
                Ok(rest) => to.join(rest),
                Err(_) => old,
            };
            self.items.insert(new, tags);
        }
        self.version += 1;
    }

    /// All items with `color`, sorted by path.
    pub fn with_tag(&self, color: TagColor) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = self
            .items
            .iter()
            .filter(|(_, t)| t.contains(&color))
            .map(|(p, _)| p.clone())
            .collect();
        v.sort();
        v
    }

    /// How many items carry each colour.
    pub fn counts(&self) -> [usize; 7] {
        let mut counts = [0; 7];
        for tags in self.items.values() {
            for t in tags {
                counts[t.index()] += 1;
            }
        }
        counts
    }

    /// Find a colour by its name or custom label (case-insensitive).
    pub fn parse(&self, name: &str) -> Option<TagColor> {
        let name = name.trim();
        TagColor::ALL.into_iter().find(|&c| {
            c.name().eq_ignore_ascii_case(name) || self.label(c).eq_ignore_ascii_case(name)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn toggle_adds_to_all_then_removes_from_all() {
        let mut s = TagStore::default();
        let (a, b) = (p("/x/a"), p("/x/b"));
        s.toggle(std::slice::from_ref(&a), TagColor::Red);
        // Not all have it yet: add to both.
        s.toggle(&[a.clone(), b.clone()], TagColor::Red);
        assert!(s.has(&a, TagColor::Red) && s.has(&b, TagColor::Red));
        s.toggle(&[a.clone(), b.clone()], TagColor::Red);
        assert!(s.tags(&a).is_empty() && s.is_empty());

        s.toggle(std::slice::from_ref(&a), TagColor::Blue);
        s.toggle(std::slice::from_ref(&a), TagColor::Red);
        assert_eq!(s.tags(&a), [TagColor::Red, TagColor::Blue]);
        assert_eq!(s.counts()[TagColor::Blue.index()], 1);
    }

    #[test]
    fn tags_follow_moves_of_items_and_folders() {
        let mut s = TagStore::default();
        s.toggle(&[p("/x/dir")], TagColor::Green);
        s.toggle(&[p("/x/dir/sub/file.txt")], TagColor::Red);
        s.toggle(&[p("/x/dir2")], TagColor::Blue);
        s.moved(&p("/x/dir"), &p("/y/renamed"));
        assert!(s.has(&p("/y/renamed"), TagColor::Green));
        assert!(s.has(&p("/y/renamed/sub/file.txt"), TagColor::Red));
        // `dir2` only shares a prefix string, not a path component.
        assert!(s.has(&p("/x/dir2"), TagColor::Blue));
        assert!(s.tags(&p("/x/dir")).is_empty());
    }

    #[test]
    fn labels_and_parsing() {
        let mut s = TagStore::default();
        assert_eq!(s.label(TagColor::Red), "Red");
        s.set_label(TagColor::Red, " Urgent ");
        assert_eq!(s.label(TagColor::Red), "Urgent");
        assert_eq!(s.parse("urgent"), Some(TagColor::Red));
        assert_eq!(s.parse("red"), Some(TagColor::Red));
        assert_eq!(s.parse("nope"), None);
    }

    #[test]
    fn round_trips_through_toml_and_prunes_missing_items() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.txt");
        std::fs::write(&real, "x").unwrap();
        let mut s = TagStore::default();
        s.toggle(
            &[real.clone(), tmp.path().join("gone.txt")],
            TagColor::Yellow,
        );
        s.set_label(TagColor::Yellow, "Later");

        let text = toml::to_string_pretty(&s.to_file()).unwrap();
        let mut back = TagStore::from_file(toml::from_str(&text).unwrap());
        assert_eq!(back.with_tag(TagColor::Yellow).len(), 2);
        back.prune();
        assert_eq!(back.with_tag(TagColor::Yellow), vec![real]);
        assert_eq!(back.label(TagColor::Yellow), "Later");
    }
}
