//! Group by: the items of a folder in groups by type, date modified, size
//! or tag (Explorer's buckets), each group collapsible, and where its
//! header and items go in the list and the grid.

use std::collections::{BTreeMap, HashSet};
use std::time::SystemTime;

use chrono::{Datelike, Days, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::entry::FileEntry;
use crate::i18n::t;
use crate::tags::TagColor;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GroupBy {
    #[default]
    None,
    Type,
    Modified,
    Size,
    Tag,
}

impl GroupBy {
    pub const ALL: [GroupBy; 5] = [
        Self::None,
        Self::Type,
        Self::Modified,
        Self::Size,
        Self::Tag,
    ];

    /// English name (translated where it is shown).
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Type => "Type",
            Self::Modified => "Date modified",
            Self::Size => "Size",
            Self::Tag => "Tag",
        }
    }
}

/// The groups of a listing, and the items shown (those of collapsed groups
/// are left out).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grouped {
    /// Indices into the entries, in display order: group by group.
    pub visible: Vec<usize>,
    pub groups: Vec<GroupRow>,
}

/// One group's header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRow {
    /// Stays the same while the group exists (collapsed groups are kept by it).
    pub id: String,
    pub label: String,
    /// Items in the group, also when it is collapsed.
    pub count: usize,
    /// Its items are `visible[start..start + len]` (none when collapsed).
    pub start: usize,
    pub len: usize,
    pub collapsed: bool,
}

/// Where an item goes: the order of its group, its id and its name.
struct Bucket {
    rank: (u32, String),
    id: String,
    label: String,
}

impl Bucket {
    fn new(rank: u32, id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            rank: (rank, String::new()),
            id: id.into(),
            label: label.into(),
        }
    }
}

/// What grouping needs to know besides the entries.
pub struct GroupContext<'a> {
    pub today: NaiveDate,
    /// The item's tags.
    pub tags: &'a dyn Fn(&FileEntry) -> Vec<TagColor>,
    /// The name of a tag (users can rename them).
    pub tag_label: &'a dyn Fn(TagColor) -> String,
}

impl GroupContext<'_> {
    pub fn today() -> NaiveDate {
        Local::now().date_naive()
    }
}

/// Put `items` (indices into `entries`, in display order) into groups.
/// Items keep their order within a group, so sorting applies inside groups.
/// Items of the groups in `collapsed` are left out of `visible`.
pub fn group(
    entries: &[FileEntry],
    items: &[usize],
    by: GroupBy,
    cx: &GroupContext<'_>,
    collapsed: &HashSet<String>,
) -> Grouped {
    if by == GroupBy::None {
        return Grouped {
            visible: items.to_vec(),
            groups: Vec::new(),
        };
    }
    let mut buckets: BTreeMap<(u32, String), (String, String, Vec<usize>)> = BTreeMap::new();
    for &i in items {
        let Some(e) = entries.get(i) else { continue };
        let b = bucket(e, by, cx);
        buckets
            .entry(b.rank)
            .or_insert_with(|| (b.id, b.label, Vec::new()))
            .2
            .push(i);
    }
    let mut out = Grouped::default();
    for (_, (id, label, members)) in buckets {
        let is_collapsed = collapsed.contains(&id);
        let start = out.visible.len();
        if !is_collapsed {
            out.visible.extend(&members);
        }
        out.groups.push(GroupRow {
            id,
            label,
            count: members.len(),
            start,
            len: if is_collapsed { 0 } else { members.len() },
            collapsed: is_collapsed,
        });
    }
    out
}

fn bucket(e: &FileEntry, by: GroupBy, cx: &GroupContext<'_>) -> Bucket {
    let folders = || Bucket::new(0, "folders", t("Folders"));
    match by {
        GroupBy::None => Bucket::new(0, "", ""),
        GroupBy::Type if e.is_dir => folders(),
        GroupBy::Type => {
            let label = e.type_label();
            let key = label.to_lowercase();
            Bucket {
                rank: (1, key.clone()),
                id: format!("type:{key}"),
                label,
            }
        }
        GroupBy::Modified => {
            let b = date_bucket(e.modified, cx.today);
            Bucket::new(b as u32, format!("date:{}", b as u32), t(b.label()))
        }
        GroupBy::Size if e.is_dir => folders(),
        GroupBy::Size => {
            let b = size_bucket(e.size);
            Bucket::new(1 + b as u32, format!("size:{}", b as u32), t(b.label()))
        }
        GroupBy::Tag => match TagColor::ALL.into_iter().find(|c| (cx.tags)(e).contains(c)) {
            Some(c) => Bucket::new(
                c.index() as u32,
                format!("tag:{}", c.index()),
                (cx.tag_label)(c),
            ),
            None => Bucket::new(100, "tag:none", t("No tag")),
        },
    }
}

/// Explorer's groups for dates, newest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DateBucket {
    Today,
    Yesterday,
    EarlierThisWeek,
    LastWeek,
    EarlierThisMonth,
    LastMonth,
    EarlierThisYear,
    LongAgo,
}

impl DateBucket {
    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Yesterday => "Yesterday",
            Self::EarlierThisWeek => "Earlier this week",
            Self::LastWeek => "Last week",
            Self::EarlierThisMonth => "Earlier this month",
            Self::LastMonth => "Last month",
            Self::EarlierThisYear => "Earlier this year",
            Self::LongAgo => "A long time ago",
        }
    }
}

/// The group of a date, as seen `today` (weeks start on Monday). Dates in
/// the future count as today, unknown ones as long ago.
pub fn date_bucket(t: Option<SystemTime>, today: NaiveDate) -> DateBucket {
    let Some(t) = t else {
        return DateBucket::LongAgo;
    };
    let d = chrono::DateTime::<Local>::from(t).date_naive();
    day_bucket(d, today)
}

fn day_bucket(d: NaiveDate, today: NaiveDate) -> DateBucket {
    let days_ago = |n: u64| today.checked_sub_days(Days::new(n)).unwrap_or(today);
    let week_start = days_ago(u64::from(today.weekday().num_days_from_monday()));
    let month_start = today.with_day(1).unwrap_or(today);
    let last_month_start = month_start
        .pred_opt()
        .and_then(|d| d.with_day(1))
        .unwrap_or(month_start);
    if d >= today {
        DateBucket::Today
    } else if d == days_ago(1) {
        DateBucket::Yesterday
    } else if d >= week_start {
        DateBucket::EarlierThisWeek
    } else if d
        >= week_start
            .checked_sub_days(Days::new(7))
            .unwrap_or(week_start)
    {
        DateBucket::LastWeek
    } else if d >= month_start {
        DateBucket::EarlierThisMonth
    } else if d >= last_month_start {
        DateBucket::LastMonth
    } else if d.year() == today.year() {
        DateBucket::EarlierThisYear
    } else {
        DateBucket::LongAgo
    }
}

/// Explorer's groups for file sizes, smallest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SizeBucket {
    Empty,
    Tiny,
    Small,
    Medium,
    Large,
    Huge,
    Gigantic,
}

impl SizeBucket {
    pub fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty (0 KB)",
            Self::Tiny => "Tiny (0 – 16 KB)",
            Self::Small => "Small (16 KB – 1 MB)",
            Self::Medium => "Medium (1 – 128 MB)",
            Self::Large => "Large (128 MB – 1 GB)",
            Self::Huge => "Huge (1 – 4 GB)",
            Self::Gigantic => "Gigantic (> 4 GB)",
        }
    }
}

pub fn size_bucket(bytes: u64) -> SizeBucket {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    match bytes {
        0 => SizeBucket::Empty,
        b if b < 16 * KB => SizeBucket::Tiny,
        b if b < MB => SizeBucket::Small,
        b if b < 128 * MB => SizeBucket::Medium,
        b if b < GB => SizeBucket::Large,
        b if b < 4 * GB => SizeBucket::Huge,
        _ => SizeBucket::Gigantic,
    }
}

// ---- where things go in the list and the grid ---------------------------------

/// A row of the grouped details list: a group header or an item (its index
/// in `visible`). Each group's header comes before its items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListRow {
    Header(usize),
    Item(usize),
}

/// Rows of the list: one per item, plus one per group header.
pub fn row_count(groups: &[GroupRow], items: usize) -> usize {
    items + groups.len()
}

/// What list row `r` shows. Without groups, row `r` is item `r`.
pub fn row_at(groups: &[GroupRow], r: usize) -> ListRow {
    // Group g's header is at row `start + g`: find the last one at or before `r`.
    let (mut lo, mut hi) = (0, groups.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        if groups[mid].start + mid <= r {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    match lo.checked_sub(1) {
        None => ListRow::Item(r),
        Some(g) if groups[g].start + g == r => ListRow::Header(g),
        Some(g) => ListRow::Item(r - g - 1),
    }
}

/// The group item `i` (in `visible`) is in.
pub fn group_of(groups: &[GroupRow], i: usize) -> Option<usize> {
    let before = groups.partition_point(|g| g.start <= i);
    (0..before)
        .rev()
        .find(|&g| groups[g].len > 0)
        .filter(|&g| i < groups[g].start + groups[g].len)
}

/// The list row of item `i`.
pub fn row_of(groups: &[GroupRow], i: usize) -> usize {
    match group_of(groups, i) {
        Some(g) => i + g + 1,
        None => i,
    }
}

/// The item `rows` rows below (or above, if negative) item `i` in a grid
/// `cols` wide, where every group starts on a new row.
pub fn grid_step(groups: &[GroupRow], cols: usize, i: usize, rows: isize) -> usize {
    let cols = cols.max(1);
    let shown: Vec<&GroupRow> = groups.iter().filter(|g| g.len > 0).collect();
    let Some(mut g) = shown
        .iter()
        .position(|x| (x.start..x.start + x.len).contains(&i))
    else {
        return i;
    };
    let (mut row, col) = ((i - shown[g].start) / cols, (i - shown[g].start) % cols);
    let rows_of = |g: &GroupRow| g.len.div_ceil(cols);
    let mut left = rows;
    while left != 0 {
        if left > 0 {
            if row + 1 < rows_of(shown[g]) {
                row += 1;
            } else if g + 1 < shown.len() {
                g += 1;
                row = 0;
            } else {
                break;
            }
            left -= 1;
        } else {
            if row > 0 {
                row -= 1;
            } else if g > 0 {
                g -= 1;
                row = rows_of(shown[g]) - 1;
            } else {
                break;
            }
            left += 1;
        }
    }
    let x = shown[g];
    x.start + (row * cols + col).min(x.len - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(name: &str, is_dir: bool, size: u64) -> FileEntry {
        FileEntry {
            path: PathBuf::from(format!("/x/{name}")),
            name: name.into(),
            is_dir,
            is_symlink: false,
            size,
            modified: None,
            created: None,
            hidden: false,
            readonly: false,
            cloud: None,
        }
    }

    fn day(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn no_tags(_: &FileEntry) -> Vec<TagColor> {
        Vec::new()
    }

    fn tag_name(c: TagColor) -> String {
        c.name().to_owned()
    }

    fn cx() -> GroupContext<'static> {
        GroupContext {
            today: day(2026, 10, 29),
            tags: &no_tags,
            tag_label: &tag_name,
        }
    }

    #[test]
    fn dates_like_explorer() {
        use DateBucket::*;
        // Thursday, 29 October 2026; the week started on Monday the 26th.
        let today = day(2026, 10, 29);
        let b = |y, m, d| day_bucket(day(y, m, d), today);
        assert_eq!(b(2026, 10, 30), Today);
        assert_eq!(b(2026, 10, 29), Today);
        assert_eq!(b(2026, 10, 28), Yesterday);
        assert_eq!(b(2026, 10, 26), EarlierThisWeek);
        assert_eq!(b(2026, 10, 25), LastWeek);
        assert_eq!(b(2026, 10, 19), LastWeek);
        assert_eq!(b(2026, 10, 18), EarlierThisMonth);
        assert_eq!(b(2026, 10, 1), EarlierThisMonth);
        assert_eq!(b(2026, 9, 30), LastMonth);
        assert_eq!(b(2026, 9, 1), LastMonth);
        assert_eq!(b(2026, 8, 31), EarlierThisYear);
        assert_eq!(b(2026, 1, 1), EarlierThisYear);
        assert_eq!(b(2025, 12, 31), LongAgo);
        assert_eq!(date_bucket(None, today), LongAgo);
        // On a Monday, yesterday was last week: it is still "Yesterday".
        let monday = day(2026, 11, 2);
        assert_eq!(day_bucket(day(2026, 11, 1), monday), Yesterday);
        assert_eq!(day_bucket(day(2026, 10, 31), monday), LastWeek);
        // In January, December is last month.
        assert_eq!(day_bucket(day(2025, 12, 20), day(2026, 1, 20)), LastMonth);
    }

    #[test]
    fn sizes_like_explorer() {
        use SizeBucket::*;
        assert_eq!(size_bucket(0), Empty);
        assert_eq!(size_bucket(1), Tiny);
        assert_eq!(size_bucket(16 * 1024 - 1), Tiny);
        assert_eq!(size_bucket(16 * 1024), Small);
        assert_eq!(size_bucket(1024 * 1024), Medium);
        assert_eq!(size_bucket(128 << 20), Large);
        assert_eq!(size_bucket(1 << 30), Huge);
        assert_eq!(size_bucket(4 << 30), Gigantic);
    }

    #[test]
    fn groups_keep_the_sort_order_and_folders_go_first() {
        let entries = vec![
            entry("b.txt", false, 100),
            entry("docs", true, 0),
            entry("a.pdf", false, 0),
            entry("c.txt", false, 5 << 20),
        ];
        let items = [0, 1, 2, 3];
        let none = HashSet::new();
        let by_type = group(&entries, &items, GroupBy::Type, &cx(), &none);
        let labels: Vec<_> = by_type.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Folders", "PDF document (PDF)", "Text document (TXT)"]
        );
        assert_eq!(by_type.visible, vec![1, 2, 0, 3]);
        let by_size = group(&entries, &items, GroupBy::Size, &cx(), &none);
        let ids: Vec<_> = by_size.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["folders", "size:0", "size:1", "size:3"]);
        // Not grouped: as they are.
        let plain = group(&entries, &items, GroupBy::None, &cx(), &none);
        assert_eq!(plain.visible, items);
        assert!(plain.groups.is_empty());
    }

    #[test]
    fn collapsed_groups_hide_their_items() {
        let entries = vec![
            entry("a.txt", false, 1),
            entry("d", true, 0),
            entry("b.txt", false, 1),
        ];
        let collapsed: HashSet<String> = ["folders".to_owned()].into();
        let g = group(&entries, &[0, 1, 2], GroupBy::Type, &cx(), &collapsed);
        assert_eq!(g.visible, vec![0, 2]);
        assert!(g.groups[0].collapsed);
        assert_eq!((g.groups[0].count, g.groups[0].len), (1, 0));
        assert_eq!((g.groups[1].start, g.groups[1].len), (0, 2));
    }

    #[test]
    fn tags_group_by_their_first_tag() {
        let entries = vec![entry("a", false, 1), entry("b", false, 1)];
        let tags = |e: &FileEntry| {
            if e.name == "a" {
                vec![TagColor::Blue, TagColor::Red]
            } else {
                Vec::new()
            }
        };
        let cx = GroupContext {
            tags: &tags,
            ..cx()
        };
        let g = group(&entries, &[0, 1], GroupBy::Tag, &cx, &HashSet::new());
        let ids: Vec<_> = g.groups.iter().map(|g| g.id.as_str()).collect();
        assert_eq!(ids, vec!["tag:0", "tag:none"]);
    }

    fn rows(spec: &[(usize, usize)]) -> Vec<GroupRow> {
        spec.iter()
            .enumerate()
            .map(|(n, &(start, len))| GroupRow {
                id: n.to_string(),
                label: String::new(),
                count: len.max(1),
                start,
                len,
                collapsed: len == 0,
            })
            .collect()
    }

    #[test]
    fn list_rows_put_headers_before_their_items() {
        // Group 0: items 0..2, group 1 collapsed, group 2: items 2..5.
        let g = rows(&[(0, 2), (2, 0), (2, 3)]);
        assert_eq!(row_count(&g, 5), 8);
        let all: Vec<ListRow> = (0..8).map(|r| row_at(&g, r)).collect();
        use ListRow::*;
        assert_eq!(
            all,
            vec![
                Header(0),
                Item(0),
                Item(1),
                Header(1),
                Header(2),
                Item(2),
                Item(3),
                Item(4)
            ]
        );
        for i in 0..5 {
            assert_eq!(row_at(&g, row_of(&g, i)), Item(i));
        }
        // No groups: rows are items.
        assert_eq!(row_at(&[], 3), Item(3));
        assert_eq!(row_of(&[], 3), 3);
    }

    #[test]
    fn grid_steps_move_between_groups() {
        // 3 columns. Group 0: items 0..5 (rows 0-2, 3-4), group 1: 5..7.
        let g = rows(&[(0, 5), (5, 2)]);
        assert_eq!(grid_step(&g, 3, 1, 1), 4);
        // From the second row's middle down into the next group.
        assert_eq!(grid_step(&g, 3, 4, 1), 6);
        // From column 2 down to a shorter row: its last item.
        assert_eq!(grid_step(&g, 3, 2, 1), 4);
        assert_eq!(grid_step(&g, 3, 2, 2), 6);
        assert_eq!(grid_step(&g, 3, 6, -1), 4);
        assert_eq!(grid_step(&g, 3, 5, -2), 0);
        // At the edges it stays.
        assert_eq!(grid_step(&g, 3, 0, -1), 0);
        assert_eq!(grid_step(&g, 3, 6, 1), 6);
    }
}
