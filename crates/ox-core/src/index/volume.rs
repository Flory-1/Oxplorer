//! How a volume's master file table and change journal are reached: through
//! the index service when it is turned on and installed, else directly when
//! Oxplorer runs as administrator. When neither works, the callers walk the
//! folders and use a file watcher.

/// A volume's folder tree, and the size and last write time of each file
/// record by record number.
#[cfg(windows)]
pub(super) type VolumeTree = (super::tree::RawTree, Vec<(u64, u32)>);

/// One way to read a volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Route {
    /// Through the index service (`helper`).
    Service,
    /// Opening `\\.\C:` ourselves (as administrator).
    Direct,
}

/// The routes to try, in order.
pub(super) fn routes(service: bool) -> &'static [Route] {
    if service {
        &[Route::Service, Route::Direct]
    } else {
        &[Route::Direct]
    }
}

/// The first route that works, with what it gave.
pub(super) fn first<T>(
    service: bool,
    mut attempt: impl FnMut(Route) -> Option<T>,
) -> Option<(T, Route)> {
    routes(service)
        .iter()
        .find_map(|&r| attempt(r).map(|t| (t, r)))
}

#[cfg(windows)]
pub(super) use windows::read_tree;

#[cfg(windows)]
mod windows {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    use super::super::helper;
    use super::super::mft::{open_volume, read_tree_direct};
    use super::super::tree::{record_number, RawTree};
    use super::{first, Route, VolumeTree};

    /// The size and date table can't be much bigger than the number of records.
    fn meta_fits(max_record: u64, records: usize) -> bool {
        max_record < 1 << 32 && max_record <= records as u64 * 4 + 1_000_000
    }

    /// The volume's tree and each record's size and date, from the service.
    fn read_tree_service(
        letter: char,
        cancel: &AtomicBool,
        progress: &AtomicU64,
    ) -> Option<VolumeTree> {
        let mut client = helper::connect()?;
        let mut tree = RawTree::default();
        let mut sized: Vec<(u64, u64, u32)> = Vec::new();
        let res = client.enumerate(letter, cancel, |r| {
            // The root lists itself as "."; nothing else may be "." or "..".
            let dots = r.name == "." || r.name == "..";
            if dots && record_number(r.frn) != record_number(r.parent) {
                return;
            }
            tree.add(r.frn, r.parent, &r.name, r.attrs);
            sized.push((record_number(r.frn), r.size, r.mtime));
            progress.fetch_add(1, Ordering::Relaxed);
        });
        let fail = || {
            progress.fetch_sub(tree.len() as u64, Ordering::Relaxed);
        };
        if let Err(e) = res {
            log::info!("index service: couldn't list {letter}: ({e:?})");
            fail();
            return None;
        }
        let max = sized.iter().map(|s| s.0).max().unwrap_or(0);
        if !meta_fits(max, sized.len()) {
            log::warn!("index service: record numbers of {letter}: out of range");
            fail();
            return None;
        }
        let mut meta = vec![(0u64, 0u32); max as usize + 1];
        for (rn, size, mtime) in sized {
            meta[rn as usize] = (size, mtime);
        }
        Some((tree, meta))
    }

    /// The tree and sizes and dates of the NTFS volume `letter`, from the
    /// first route that works (the progress counts its records).
    pub(in crate::index) fn read_tree(
        letter: char,
        cancel: &AtomicBool,
        progress: &AtomicU64,
    ) -> Option<(VolumeTree, Route)> {
        first(helper::enabled(), |route| match route {
            Route::Service => read_tree_service(letter, cancel, progress),
            Route::Direct => {
                open_volume(letter).and_then(|v| read_tree_direct(&v, cancel, progress))
            }
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn size_table_stays_bounded() {
            assert!(meta_fits(0, 0));
            assert!(meta_fits(3_000_000, 1_000_000));
            assert!(!meta_fits(1 << 40, 1_000_000));
            assert!(!meta_fits(50_000_000, 1000));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tries_the_service_first_then_direct_then_gives_up() {
        let mut tried = Vec::new();
        let got = first(true, |r| {
            tried.push(r);
            (r == Route::Direct).then_some("direct")
        });
        assert_eq!(got, Some(("direct", Route::Direct)));
        assert_eq!(tried, vec![Route::Service, Route::Direct]);

        // The service answers: direct isn't tried.
        let mut tried = Vec::new();
        let got = first(true, |r| {
            tried.push(r);
            Some(r)
        });
        assert_eq!(got, Some((Route::Service, Route::Service)));
        assert_eq!(tried, vec![Route::Service]);

        // Turned off: only direct.
        let mut tried = Vec::new();
        let got: Option<((), Route)> = first(false, |r| {
            tried.push(r);
            None
        });
        assert_eq!(got, None, "nothing works: the caller walks");
        assert_eq!(tried, vec![Route::Direct]);
    }
}
