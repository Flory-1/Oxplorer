//! Rebuilding the folder tree from a flat list of (file reference, parent
//! reference, name) records, as the NTFS master file table lists them.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use super::{is_excluded, Builder, F_DIR, F_HIDDEN};

pub(super) const ATTR_HIDDEN: u32 = 0x2;
pub(super) const ATTR_DIRECTORY: u32 = 0x10;
pub(super) const ATTR_REPARSE_POINT: u32 = 0x400;

/// The record number part of a file reference (the top 16 bits are a sequence number).
pub(super) fn record_number(frn: u64) -> u64 {
    frn & 0xFFFF_FFFF_FFFF
}

struct Node {
    frn: u64,
    parent: u64,
    name_off: u32,
    name_len: u16,
    attrs: u32,
}

/// All files and folders of a volume, in no particular order.
#[derive(Default)]
pub(super) struct RawTree {
    nodes: Vec<Node>,
    names: String,
}

impl RawTree {
    pub fn add(&mut self, frn: u64, parent: u64, name: &str, attrs: u32) {
        let mut len = name.len().min(u16::MAX as usize);
        while !name.is_char_boundary(len) {
            len -= 1;
        }
        self.nodes.push(Node {
            frn,
            parent,
            name_off: self.names.len() as u32,
            name_len: len as u16,
            attrs,
        });
        self.names.push_str(&name[..len]);
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    fn name(&self, n: &Node) -> &str {
        let start = n.name_off as usize;
        &self.names[start..start + n.name_len as usize]
    }

    /// Push everything below the folder `root` into `b`, attached to entry
    /// `root_idx`, parents before children. Excluded names are skipped with
    /// everything in them; hidden folders hide their contents, as in the
    /// walk. `meta` gives the size and date (seconds since 1970) of a file
    /// reference. Returns how many entries were pushed.
    #[allow(clippy::too_many_arguments)]
    pub fn push_into(
        &self,
        root: u64,
        root_idx: u32,
        exclude: &[String],
        b: &mut Builder,
        meta: impl Fn(u64) -> (u64, u32),
        cancel: &AtomicBool,
        progress: &AtomicU64,
    ) -> usize {
        let mut by_parent: Vec<u32> = (0..self.nodes.len() as u32).collect();
        by_parent.sort_unstable_by_key(|&i| self.nodes[i as usize].parent);
        let children = |frn: u64| {
            let lo = by_parent.partition_point(|&i| self.nodes[i as usize].parent < frn);
            let hi = by_parent.partition_point(|&i| self.nodes[i as usize].parent <= frn);
            &by_parent[lo..hi]
        };

        let mut pushed = 0;
        let mut queue = VecDeque::from([(root, root_idx, false)]);
        while let Some((frn, idx, hidden)) = queue.pop_front() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let mut count = 0;
            for &k in children(frn) {
                let n = &self.nodes[k as usize];
                // The root folder is its own parent.
                if n.frn == frn {
                    continue;
                }
                let name = self.name(n);
                // NTFS metadata files ($MFT, $Extend, …) aren't listed by the walk either.
                if name.starts_with('$') && record_number(n.frn) < 64 {
                    continue;
                }
                if is_excluded(name, exclude) {
                    continue;
                }
                // Linked folders (junctions, symbolic links) have no children of their
                // own in the table; like the walk, they are indexed but not as folders.
                let is_dir = n.attrs & ATTR_DIRECTORY != 0
                    && (n.attrs & ATTR_REPARSE_POINT == 0 || !children(n.frn).is_empty());
                let h = hidden || n.attrs & ATTR_HIDDEN != 0;
                let mut flags = 0;
                if is_dir {
                    flags |= F_DIR;
                }
                if h {
                    flags |= F_HIDDEN;
                }
                let (size, mtime) = meta(n.frn);
                let size = if is_dir { 0 } else { size };
                let i = b.push(name, idx, flags, size, mtime);
                count += 1;
                if is_dir {
                    queue.push_back((n.frn, i, h));
                }
            }
            pushed += count;
            progress.fetch_add(count as u64, Ordering::Relaxed);
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::super::NO_PARENT;
    use super::*;

    const DIR: u32 = ATTR_DIRECTORY;
    const ROOT: u64 = 5 | 5 << 48;

    fn paths(b: &Builder) -> Vec<(String, u8, u64, u32)> {
        let mut v: Vec<_> = (0..b.entries.len())
            .map(|i| {
                let mut parts = Vec::new();
                let mut cur = i as u32;
                while cur != NO_PARENT {
                    let e = &b.entries[cur as usize];
                    let start = e.name_off as usize;
                    parts.push(&b.names[start..start + e.name_len as usize]);
                    cur = e.parent;
                }
                parts.reverse();
                let e = &b.entries[i];
                (parts.join("/"), e.flags, e.size(), e.mtime)
            })
            .collect();
        v.sort();
        v
    }

    #[test]
    fn rebuilds_the_tree_from_parent_references() {
        let mut t = RawTree::default();
        // Out of order, as the table lists them: children before parents.
        t.add(40, 30, "deep.txt", 0);
        t.add(ROOT, ROOT, ".", DIR | ATTR_HIDDEN);
        t.add(20, ROOT, "Users", DIR);
        t.add(30, 20, "me", DIR);
        t.add(31, 20, "Secret", DIR | ATTR_HIDDEN);
        t.add(32, 31, "key.txt", 0);
        t.add(33, ROOT, "node_modules", DIR);
        t.add(34, 33, "pkg.js", 0);
        t.add(6, ROOT, "$MFT", ATTR_HIDDEN);
        t.add(11, ROOT, "$Extend", DIR | ATTR_HIDDEN);
        t.add(50, 11, "$UsnJrnl", 0);
        t.add(70, ROOT, "$Recycle.Bin", DIR | ATTR_HIDDEN);
        // A junction (no children here) and a cloud folder (has children).
        t.add(36, 20, "Default User", DIR | ATTR_REPARSE_POINT);
        t.add(37, 30, "OneDrive", DIR | ATTR_REPARSE_POINT);
        t.add(38, 37, "doc.docx", 0);
        // Orphans whose folder isn't listed are left out.
        t.add(60, 999, "orphan.txt", 0);
        assert_eq!(t.len(), 16);

        let mut b = Builder::default();
        let root = b.push("C:\\", NO_PARENT, F_DIR, 0, 0);
        let pushed = t.push_into(
            ROOT,
            root,
            &["node_modules".to_owned()],
            &mut b,
            |frn| (frn * 10, frn as u32),
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        );
        assert_eq!(pushed, 9);
        assert_eq!(
            paths(&b),
            vec![
                ("C:\\".into(), F_DIR, 0, 0),
                ("C:\\/$Recycle.Bin".into(), F_DIR | F_HIDDEN, 0, 70),
                ("C:\\/Users".into(), F_DIR, 0, 20),
                ("C:\\/Users/Default User".into(), 0, 360, 36),
                ("C:\\/Users/Secret".into(), F_DIR | F_HIDDEN, 0, 31),
                ("C:\\/Users/Secret/key.txt".into(), F_HIDDEN, 320, 32),
                ("C:\\/Users/me".into(), F_DIR, 0, 30),
                ("C:\\/Users/me/OneDrive".into(), F_DIR, 0, 37),
                ("C:\\/Users/me/OneDrive/doc.docx".into(), 0, 380, 38),
                ("C:\\/Users/me/deep.txt".into(), 0, 400, 40),
            ]
        );
        // Parents come before their children.
        for (i, e) in b.entries.iter().enumerate() {
            assert!(e.parent == NO_PARENT || (e.parent as usize) < i);
        }
    }

    #[test]
    fn cancelled_stops_early() {
        let mut t = RawTree::default();
        t.add(20, ROOT, "a", DIR);
        let mut b = Builder::default();
        let root = b.push("C:\\", NO_PARENT, F_DIR, 0, 0);
        let n = t.push_into(
            ROOT,
            root,
            &[],
            &mut b,
            |_| (0, 0),
            &AtomicBool::new(true),
            &AtomicU64::new(0),
        );
        assert_eq!(n, 0);
    }
}
