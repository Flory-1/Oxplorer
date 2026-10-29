//! Group by month: "October 2026", newest first (the Gallery's groups).

use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local};

use super::Bucket;
use crate::i18n::t;

/// English month names (translated where they are shown).
pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// "October 2026" (in the interface language). `month` is 1–12.
pub fn month_label(year: i32, month: u32) -> String {
    let name = MONTHS
        .get(month.wrapping_sub(1) as usize)
        .map_or("", |m| t(m));
    format!("{name} {year}")
}

/// The group of a date (local time); unknown dates come last.
pub(super) fn bucket(date: Option<SystemTime>) -> Bucket {
    let Some(date) = date else {
        return Bucket::new(u32::MAX, "month:none", t("Date unknown"));
    };
    let d = DateTime::<Local>::from(date);
    let (year, month) = (d.year(), d.month());
    // Months since year 0; newer months get smaller ranks (come first).
    let n = i64::from(year) * 12 + i64::from(month) - 1;
    let rank = (i64::from(u32::MAX - 1) - n).clamp(0, i64::from(u32::MAX - 1)) as u32;
    Bucket::new(
        rank,
        format!("month:{year:04}-{month:02}"),
        month_label(year, month),
    )
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use chrono::TimeZone;

    fn at(y: i32, m: u32, d: u32, h: u32) -> SystemTime {
        Local
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .single()
            .expect("a valid local time")
            .into()
    }

    fn file(name: &str, modified: Option<SystemTime>) -> FileEntry {
        FileEntry {
            path: name.into(),
            name: name.into(),
            is_dir: false,
            is_symlink: false,
            size: 1,
            modified,
            created: None,
            hidden: false,
            readonly: false,
            cloud: None,
        }
    }

    #[test]
    fn labels_name_the_month_and_year() {
        assert_eq!(month_label(2026, 10), "October 2026");
        assert_eq!(month_label(2025, 1), "January 2025");
        assert_eq!(month_label(2025, 12), "December 2025");
    }

    #[test]
    fn months_newest_first_by_date_taken_else_modified() {
        let entries = vec![
            // Taken in March 2024, copied (modified) in October 2026.
            file("copied.jpg", Some(at(2026, 10, 2, 12))),
            file("new.jpg", Some(at(2026, 10, 28, 9))),
            file("start.jpg", Some(at(2026, 9, 1, 0))),
            file("end.jpg", Some(at(2026, 8, 31, 23))),
            file("old.jpg", Some(at(2025, 12, 31, 12))),
            file("never.jpg", None),
        ];
        let taken = |e: &FileEntry| (e.name == "copied.jpg").then(|| at(2024, 3, 15, 10));
        let no_tags = |_: &FileEntry| Vec::new();
        let tag_label = |c: TagColor| c.name().to_owned();
        let cx = GroupContext {
            today: NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
            tags: &no_tags,
            tag_label: &tag_label,
            date_taken: &taken,
        };
        let items: Vec<usize> = (0..entries.len()).collect();
        let g = group(&entries, &items, GroupBy::Month, &cx, &HashSet::new());
        let labels: Vec<_> = g.groups.iter().map(|g| g.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "October 2026",
                "September 2026",
                "August 2026",
                "December 2025",
                "March 2024",
                "Date unknown"
            ]
        );
        // Items keep their order within a month.
        assert_eq!(g.visible, [1, 2, 3, 4, 0, 5]);
        assert_eq!(g.groups[0].id, "month:2026-10");
        assert_eq!(g.groups[4].id, "month:2024-03");
    }
}
