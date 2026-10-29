//! The columns of the details view: which there are, which are shown, in
//! what order and how wide, and how they share the width of a pane.
//!
//! To add a column: add a [`ColumnId`] variant, describe it in
//! [`ColumnId::info`] and put it into [`ColumnId::ALL`] (where it goes in the
//! default order), then draw its cell in the app (`ui/files/cells.rs`).

use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::entry::SortKey;

/// One column of the details view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnId {
    Name,
    /// OneDrive / Dropbox state (in cloud folders only).
    Status,
    /// Where a search result or a deleted item is (search and Recycle Bin only).
    Folder,
    /// Git state (inside repositories only).
    Git,
    Modified,
    Created,
    Type,
    Size,
    /// Width × height of pictures and videos.
    Dimensions,
    /// Playing time of audio and video files.
    Length,
    DateTaken,
    Authors,
    /// File version of programs and libraries.
    Version,
    /// Attribute letters (R H S A C E …) as in Explorer.
    Attributes,
}

/// What there is to know about a column.
pub struct ColumnInfo {
    /// Name in the settings files.
    pub key: &'static str,
    /// English header text (translated where it is shown).
    pub label: &'static str,
    pub width: f32,
    /// Shown unless the user hides it.
    pub shown: bool,
    pub align_right: bool,
    /// Clicking the header sorts by this.
    pub sort: Option<SortKey>,
    /// Its values are read from the file in the background (see `props`).
    pub props: bool,
}

const fn col(key: &'static str, label: &'static str, width: f32, shown: bool) -> ColumnInfo {
    ColumnInfo {
        key,
        label,
        width,
        shown,
        align_right: false,
        sort: None,
        props: false,
    }
}

impl ColumnId {
    /// Every column, in the default order.
    pub const ALL: [ColumnId; 14] = [
        Self::Name,
        Self::Status,
        Self::Folder,
        Self::Git,
        Self::Modified,
        Self::Created,
        Self::Type,
        Self::Size,
        Self::Dimensions,
        Self::Length,
        Self::DateTaken,
        Self::Authors,
        Self::Version,
        Self::Attributes,
    ];

    pub fn info(self) -> ColumnInfo {
        let sorted = |c: ColumnInfo, key| ColumnInfo {
            sort: Some(key),
            ..c
        };
        let read = |c: ColumnInfo| ColumnInfo { props: true, ..c };
        match self {
            Self::Name => sorted(col("Name", "Name", 0.0, true), SortKey::Name),
            Self::Status => sorted(col("Status", "Status", 60.0, true), SortKey::Status),
            Self::Folder => col("Folder", "Folder", 260.0, true),
            Self::Git => col("Git", "Git", 44.0, true),
            Self::Modified => sorted(
                col("Modified", "Date modified", 140.0, true),
                SortKey::Modified,
            ),
            Self::Created => sorted(
                col("Created", "Date created", 140.0, false),
                SortKey::Created,
            ),
            Self::Type => sorted(col("Type", "Type", 150.0, true), SortKey::Type),
            Self::Size => ColumnInfo {
                align_right: true,
                ..sorted(col("Size", "Size", 90.0, true), SortKey::Size)
            },
            // Sorted once their values are read (unknown ones last).
            Self::Dimensions => sorted(
                read(col("Dimensions", "Dimensions", 110.0, false)),
                SortKey::Dimensions,
            ),
            Self::Length => sorted(read(col("Length", "Length", 80.0, false)), SortKey::Length),
            Self::DateTaken => sorted(
                read(col("DateTaken", "Date taken", 140.0, false)),
                SortKey::DateTaken,
            ),
            Self::Authors => sorted(
                read(col("Authors", "Authors", 150.0, false)),
                SortKey::Authors,
            ),
            Self::Version => sorted(
                read(col("Version", "File version", 120.0, false)),
                SortKey::Version,
            ),
            Self::Attributes => read(col("Attributes", "Attributes", 80.0, false)),
        }
    }

    pub fn key(self) -> &'static str {
        self.info().key
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.key() == key)
    }

    /// English header text.
    pub fn label(self) -> &'static str {
        self.info().label
    }

    /// Whether the column makes sense here (Folder in search results and the
    /// Recycle Bin, Git inside repositories).
    pub fn available(self, cx: ColumnContext) -> bool {
        match self {
            Self::Folder => cx.folder,
            Self::Git => cx.git,
            Self::Status => cx.cloud,
            _ => true,
        }
    }
}

impl Serialize for ColumnId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for ColumnId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let key = String::deserialize(d)?;
        Self::from_key(&key)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown column {key:?}")))
    }
}

/// Where the columns are shown.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ColumnContext {
    /// Search results, libraries, the Gallery, the flat view or the Recycle
    /// Bin: the Folder column applies.
    pub folder: bool,
    /// Inside a Git repository: the Git column applies.
    pub git: bool,
    /// Items with a cloud state (OneDrive, Dropbox, …): the Status column applies.
    pub cloud: bool,
}

/// A width the user gave a column.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ColumnWidth {
    pub id: ColumnId,
    pub width: f32,
}

/// Which columns are shown, in which order, and how wide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColumnLayout {
    /// Shown columns, in order. Name is always first.
    #[serde(deserialize_with = "known_columns")]
    pub shown: Vec<ColumnId>,
    /// Widths the user set; the others have their default width.
    #[serde(deserialize_with = "known_widths")]
    pub widths: Vec<ColumnWidth>,
}

/// Narrowest a column can be made.
pub const MIN_WIDTH: f32 = 36.0;
pub const MAX_WIDTH: f32 = 800.0;
/// Name keeps at least this much before other columns are left out.
pub const MIN_NAME_WIDTH: f32 = 180.0;

impl Default for ColumnLayout {
    fn default() -> Self {
        Self {
            shown: ColumnId::ALL
                .into_iter()
                .filter(|c| c.info().shown)
                .collect(),
            widths: Vec::new(),
        }
    }
}

/// Columns of a settings file, without the ones this version doesn't know
/// (e.g. written by a newer one).
fn known_columns<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ColumnId>, D::Error> {
    let keys = Vec::<String>::deserialize(d)?;
    Ok(keys.iter().filter_map(|k| ColumnId::from_key(k)).collect())
}

fn known_widths<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ColumnWidth>, D::Error> {
    #[derive(Deserialize)]
    struct Raw {
        id: String,
        width: f32,
    }
    let raw = Vec::<Raw>::deserialize(d)?;
    Ok(raw
        .into_iter()
        .filter_map(|r| {
            Some(ColumnWidth {
                id: ColumnId::from_key(&r.id)?,
                width: r.width,
            })
        })
        .collect())
}

impl ColumnLayout {
    /// Fix what a hand-edited or old file may have got wrong: duplicates,
    /// Name missing or not first, widths out of range.
    pub fn normalize(&mut self) {
        let mut seen = HashSet::new();
        self.shown
            .retain(|&c| c != ColumnId::Name && seen.insert(c));
        self.shown.insert(0, ColumnId::Name);
        let mut seen = HashSet::new();
        self.widths
            .retain(|w| w.id != ColumnId::Name && w.width.is_finite() && seen.insert(w.id));
        for w in &mut self.widths {
            w.width = w.width.clamp(MIN_WIDTH, MAX_WIDTH);
        }
    }

    pub fn is_shown(&self, id: ColumnId) -> bool {
        self.shown.contains(&id)
    }

    pub fn width(&self, id: ColumnId) -> f32 {
        self.widths
            .iter()
            .find(|w| w.id == id)
            .map_or(id.info().width, |w| w.width)
    }

    pub fn set_width(&mut self, id: ColumnId, width: f32) {
        if id == ColumnId::Name {
            return;
        }
        let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
        match self.widths.iter_mut().find(|w| w.id == id) {
            Some(w) => w.width = width,
            None => self.widths.push(ColumnWidth { id, width }),
        }
    }

    /// Show a hidden column (where it goes in the default order) or hide a
    /// shown one. Name can't be hidden.
    pub fn toggle(&mut self, id: ColumnId) {
        if id == ColumnId::Name {
            return;
        }
        if let Some(i) = self.shown.iter().position(|&c| c == id) {
            self.shown.remove(i);
            return;
        }
        let rank = |c: ColumnId| ColumnId::ALL.iter().position(|&a| a == c);
        let at = self
            .shown
            .iter()
            .position(|&c| rank(c) > rank(id))
            .unwrap_or(self.shown.len());
        self.shown.insert(at.max(1), id);
    }

    /// Move a shown column in front of `before` (to the end if `None`).
    /// Name stays first.
    pub fn move_before(&mut self, id: ColumnId, before: Option<ColumnId>) {
        if id == ColumnId::Name || before == Some(id) || !self.is_shown(id) {
            return;
        }
        self.shown.retain(|&c| c != id);
        let at = before
            .and_then(|b| self.shown.iter().position(|&c| c == b))
            .unwrap_or(self.shown.len());
        self.shown.insert(at.max(1), id);
    }

    /// The shown columns that apply here, in order.
    pub fn visible(&self, cx: ColumnContext) -> impl Iterator<Item = ColumnId> + '_ {
        self.shown.iter().copied().filter(move |c| c.available(cx))
    }

    /// Some shown column needs values read from the files.
    pub fn wants_props(&self, cx: ColumnContext) -> bool {
        self.visible(cx).any(|c| c.info().props)
    }

    /// The visible columns and their widths in a pane `width` wide: Name
    /// takes what the others leave. When that is too little, columns are
    /// left out (the extra ones first, then Type, Folder and the date);
    /// Git and Size always stay.
    pub fn fit(&self, cx: ColumnContext, width: f32) -> Vec<(ColumnId, f32)> {
        const LEAVE_OUT: [ColumnId; 11] = [
            ColumnId::Attributes,
            ColumnId::Status,
            ColumnId::Version,
            ColumnId::Authors,
            ColumnId::DateTaken,
            ColumnId::Length,
            ColumnId::Dimensions,
            ColumnId::Created,
            ColumnId::Type,
            ColumnId::Folder,
            ColumnId::Modified,
        ];
        let avail = width - 16.0;
        let mut cols: Vec<(ColumnId, f32)> = self
            .visible(cx)
            .map(|c| {
                (
                    c,
                    if c == ColumnId::Name {
                        0.0
                    } else {
                        self.width(c)
                    },
                )
            })
            .collect();
        let rest = |cols: &[(ColumnId, f32)]| avail - cols.iter().map(|c| c.1).sum::<f32>();
        for id in LEAVE_OUT {
            let short = MIN_NAME_WIDTH - rest(&cols);
            if short <= 0.0 {
                break;
            }
            let Some(i) = cols.iter().position(|c| c.0 == id) else {
                continue;
            };
            // The folder column gets narrower before it goes.
            if id == ColumnId::Folder && cols[i].1 - short >= 120.0 {
                cols[i].1 -= short;
            } else {
                cols.remove(i);
            }
        }
        let others = cols.iter().map(|c| c.1).sum::<f32>();
        let name = rest(&cols)
            .max(MIN_NAME_WIDTH.min(avail - others))
            .max(60.0);
        if let Some(c) = cols.iter_mut().find(|c| c.0 == ColumnId::Name) {
            c.1 = name;
        }
        cols
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERYWHERE: ColumnContext = ColumnContext {
        folder: true,
        git: true,
        cloud: true,
    };

    fn ids(cols: &[(ColumnId, f32)]) -> Vec<ColumnId> {
        cols.iter().map(|c| c.0).collect()
    }

    #[test]
    fn keys_are_unique_and_round_trip() {
        for c in ColumnId::ALL {
            assert_eq!(ColumnId::from_key(c.key()), Some(c));
            let n = ColumnId::ALL.iter().filter(|o| o.key() == c.key()).count();
            assert_eq!(n, 1, "{c:?}");
        }
        assert_eq!(ColumnId::ALL[0], ColumnId::Name);
    }

    #[test]
    fn defaults_are_the_classic_columns() {
        let l = ColumnLayout::default();
        use ColumnId::*;
        assert_eq!(
            l.shown,
            vec![Name, Status, Folder, Git, Modified, Type, Size]
        );
        let here = ColumnContext::default();
        assert_eq!(
            l.visible(here).collect::<Vec<_>>(),
            vec![Name, Modified, Type, Size]
        );
        assert!(!l.wants_props(here));
    }

    #[test]
    fn toggling_puts_columns_where_they_belong() {
        use ColumnId::*;
        let mut l = ColumnLayout::default();
        l.toggle(Created);
        assert_eq!(
            l.shown,
            vec![Name, Status, Folder, Git, Modified, Created, Type, Size]
        );
        l.toggle(Attributes);
        assert_eq!(*l.shown.last().unwrap(), Attributes);
        assert!(l.wants_props(ColumnContext::default()));
        l.toggle(Created);
        assert!(!l.is_shown(Created));
        // Name stays.
        l.toggle(Name);
        assert_eq!(l.shown[0], Name);
    }

    #[test]
    fn moving_keeps_name_first() {
        use ColumnId::*;
        let mut l = ColumnLayout::default();
        l.move_before(Size, Some(Name));
        assert_eq!(l.shown[..2], [Name, Size]);
        l.move_before(Size, None);
        assert_eq!(*l.shown.last().unwrap(), Size);
        l.move_before(Type, Some(Modified));
        assert_eq!(
            l.shown,
            vec![Name, Status, Folder, Git, Type, Modified, Size]
        );
        l.move_before(Name, None);
        assert_eq!(l.shown[0], Name);
        // Hidden columns aren't moved in.
        l.move_before(Created, None);
        assert!(!l.is_shown(Created));
    }

    #[test]
    fn widths_are_clamped_and_remembered() {
        let mut l = ColumnLayout::default();
        assert_eq!(l.width(ColumnId::Size), 90.0);
        l.set_width(ColumnId::Size, 5.0);
        assert_eq!(l.width(ColumnId::Size), MIN_WIDTH);
        l.set_width(ColumnId::Size, 120.0);
        assert_eq!(l.width(ColumnId::Size), 120.0);
        assert_eq!(l.widths.len(), 1);
    }

    #[test]
    fn narrow_panes_leave_columns_out() {
        use ColumnId::*;
        let mut l = ColumnLayout::default();
        let here = ColumnContext::default();
        let wide = l.fit(here, 1200.0);
        assert_eq!(ids(&wide), vec![Name, Modified, Type, Size]);
        // Everything adds up to the width (less the margin).
        assert_eq!(wide.iter().map(|c| c.1).sum::<f32>(), 1200.0 - 16.0);
        // Type goes first, then the date; Size stays.
        assert_eq!(ids(&l.fit(here, 500.0)), vec![Name, Modified, Size]);
        assert_eq!(ids(&l.fit(here, 300.0)), vec![Name, Size]);
        // Extra columns go before Type.
        l.toggle(Dimensions);
        assert_eq!(ids(&l.fit(here, 650.0)), vec![Name, Modified, Type, Size]);
        // The folder column of search results shrinks before it goes.
        let found = ColumnLayout::default().fit(EVERYWHERE, 666.0);
        let folder = found.iter().find(|c| c.0 == Folder).unwrap().1;
        assert!((120.0..260.0).contains(&folder));
    }

    #[test]
    fn settings_files_with_unknown_columns_still_load() {
        let text = "shown = [\"Size\", \"Name\", \"Rating\", \"Size\"]\n\
                    [[widths]]\nid = \"Size\"\nwidth = 3000.0\n\
                    [[widths]]\nid = \"Rating\"\nwidth = 80.0\n";
        let mut l: ColumnLayout = toml::from_str(text).unwrap();
        l.normalize();
        assert_eq!(l.shown, vec![ColumnId::Name, ColumnId::Size]);
        assert_eq!(l.width(ColumnId::Size), MAX_WIDTH);
        // And a round trip.
        let back: ColumnLayout = toml::from_str(&toml::to_string(&l).unwrap()).unwrap();
        assert_eq!(back, l);
        // Missing = defaults.
        let empty: ColumnLayout = toml::from_str("").unwrap();
        assert_eq!(empty, ColumnLayout::default());
    }
}
