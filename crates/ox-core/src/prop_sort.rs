//! Sorting by the values read from the files (the extra details columns:
//! dimensions, length, date taken, authors, version). They are read in the
//! background, so some items don't have theirs yet: those come last, in
//! both directions, and move into place as their values arrive.

use std::cmp::Ordering;

use crate::entry::{SortKey, SortOrder};
use crate::natural::natural_cmp;
use crate::props::Props;

impl SortKey {
    /// Sorts by a value read from the file (see [`crate::props`]).
    pub fn reads_props(self) -> bool {
        matches!(
            self,
            Self::Dimensions | Self::Length | Self::DateTaken | Self::Authors | Self::Version
        )
    }
}

/// Where an item goes for a props key: with its value, read but without
/// one (a text file has no dimensions), not read yet.
fn rank(key: SortKey, p: Option<&Props>) -> u8 {
    match p {
        None => 2,
        Some(p) if has_value(key, p) => 0,
        Some(_) => 1,
    }
}

fn has_value(key: SortKey, p: &Props) -> bool {
    let text = |s: &Option<String>| s.as_deref().is_some_and(|s| !s.trim().is_empty());
    match key {
        SortKey::Dimensions => p.dimensions.is_some(),
        SortKey::Length => p.duration.is_some(),
        SortKey::DateTaken => p.date_taken.is_some(),
        SortKey::Authors => text(&p.authors),
        SortKey::Version => text(&p.version),
        _ => false,
    }
}

/// Compare two items by a props key (`None`: not read yet). Items without
/// a value come after those with one whatever the `order`; between
/// themselves (and for equal values) they are `Equal`, for the caller to
/// order by name.
pub fn compare(key: SortKey, a: Option<&Props>, b: Option<&Props>, order: SortOrder) -> Ordering {
    let by_rank = rank(key, a).cmp(&rank(key, b));
    if by_rank != Ordering::Equal {
        return by_rank;
    }
    let (Some(a), Some(b)) = (a, b) else {
        return Ordering::Equal;
    };
    if !has_value(key, a) {
        return Ordering::Equal;
    }
    let ord = values(key, a, b);
    match order {
        SortOrder::Ascending => ord,
        SortOrder::Descending => ord.reverse(),
    }
}

/// Ascending order of two values that are both there.
fn values(key: SortKey, a: &Props, b: &Props) -> Ordering {
    let text = |s: &Option<String>| s.clone().unwrap_or_default();
    match key {
        SortKey::Dimensions => dimensions_cmp(a.dimensions, b.dimensions),
        SortKey::Length => a.duration.cmp(&b.duration),
        SortKey::DateTaken => a.date_taken.cmp(&b.date_taken),
        SortKey::Authors => natural_cmp(&text(&a.authors), &text(&b.authors)),
        SortKey::Version => version_cmp(&text(&a.version), &text(&b.version)),
        _ => Ordering::Equal,
    }
}

/// By pixel count, then by width (a wide picture before a tall one of the
/// same size).
pub fn dimensions_cmp(a: Option<(u32, u32)>, b: Option<(u32, u32)>) -> Ordering {
    let px = |d: Option<(u32, u32)>| d.map(|(w, h)| (u64::from(w) * u64::from(h), w));
    px(a).cmp(&px(b))
}

/// Version numbers part by part, numbers as numbers: `1.9` < `1.10` <
/// `2.0`, and `1.2` < `1.2.1`.
pub fn version_cmp(a: &str, b: &str) -> Ordering {
    let parts = |s: &str| {
        s.trim()
            .split(['.', ',', ' '])
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    let (pa, pb) = (parts(a), parts(b));
    for (x, y) in pa.iter().zip(&pb) {
        let ord = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            _ => natural_cmp(x, y),
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    pa.len().cmp(&pb.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn dims(w: u32, h: u32) -> Props {
        Props {
            dimensions: Some((w, h)),
            ..Default::default()
        }
    }

    #[test]
    fn unknown_values_come_last_in_both_directions() {
        let small = dims(10, 10);
        let big = dims(100, 100);
        let none = Props::default();
        for order in [SortOrder::Ascending, SortOrder::Descending] {
            let k = SortKey::Dimensions;
            // Not read yet: after everything.
            assert_eq!(compare(k, Some(&small), None, order), Ordering::Less);
            assert_eq!(compare(k, None, Some(&big), order), Ordering::Greater);
            assert_eq!(compare(k, Some(&none), None, order), Ordering::Less);
            // Read without a value: after those with one.
            assert_eq!(
                compare(k, Some(&none), Some(&small), order),
                Ordering::Greater
            );
            assert_eq!(compare(k, None, None, order), Ordering::Equal);
            assert_eq!(compare(k, Some(&none), Some(&none), order), Ordering::Equal);
        }
        let k = SortKey::Dimensions;
        assert_eq!(
            compare(k, Some(&small), Some(&big), SortOrder::Ascending),
            Ordering::Less
        );
        assert_eq!(
            compare(k, Some(&small), Some(&big), SortOrder::Descending),
            Ordering::Greater
        );
    }

    #[test]
    fn dimensions_by_pixel_count() {
        // 1000×10 has fewer pixels than 200×200.
        assert_eq!(
            dimensions_cmp(Some((1000, 10)), Some((200, 200))),
            Ordering::Less
        );
        assert_eq!(
            dimensions_cmp(Some((20, 10)), Some((10, 20))),
            Ordering::Greater
        );
        assert_eq!(dimensions_cmp(Some((5, 5)), Some((5, 5))), Ordering::Equal);
    }

    #[test]
    fn lengths_dates_and_authors() {
        let len = |s| Props {
            duration: Some(Duration::from_secs(s)),
            ..Default::default()
        };
        let asc = SortOrder::Ascending;
        assert_eq!(
            compare(SortKey::Length, Some(&len(59)), Some(&len(61)), asc),
            Ordering::Less
        );
        let taken = |s| Props {
            date_taken: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
            ..Default::default()
        };
        assert_eq!(
            compare(SortKey::DateTaken, Some(&taken(5)), Some(&taken(2)), asc),
            Ordering::Greater
        );
        let by = |s: &str| Props {
            authors: Some(s.into()),
            ..Default::default()
        };
        assert_eq!(
            compare(SortKey::Authors, Some(&by("anna")), Some(&by("Bert")), asc),
            Ordering::Less
        );
        assert_eq!(
            compare(
                SortKey::Authors,
                Some(&by("Team 2")),
                Some(&by("Team 10")),
                asc
            ),
            Ordering::Less
        );
        // An empty author counts as none.
        assert_eq!(
            compare(SortKey::Authors, Some(&by(" ")), Some(&by("Zoe")), asc),
            Ordering::Greater
        );
    }

    #[test]
    fn versions_part_by_part() {
        assert_eq!(version_cmp("1.9", "1.10"), Ordering::Less);
        assert_eq!(version_cmp("1.10", "2.0"), Ordering::Less);
        assert_eq!(
            version_cmp("10.0.19041.1", "6.1.7600.16385"),
            Ordering::Greater
        );
        assert_eq!(version_cmp("1.2", "1.2.1"), Ordering::Less);
        assert_eq!(version_cmp("1.2.0", "1.2.0"), Ordering::Equal);
        assert_eq!(version_cmp("1.0 beta", "1.0 rc"), Ordering::Less);
        assert_eq!(version_cmp("3, 1, 0, 2", "3.1.0.10"), Ordering::Less);
    }

    #[test]
    fn only_the_read_columns_read_props() {
        assert!(SortKey::Dimensions.reads_props() && SortKey::Version.reads_props());
        assert!(!SortKey::Name.reads_props() && !SortKey::Status.reads_props());
    }
}
