//! What each details column shows for an item. A new column needs an arm
//! in [`cell`] (and its description in `ox_core::columns`).

use super::*;
use ox_core::columns::ColumnId;
use ox_core::props::{self, Props};
use std::time::SystemTime;

/// The content of one cell.
pub(super) enum Cell {
    Text(String),
    /// A short coloured marker, e.g. the Git state letter.
    Symbol(&'static str, Color32),
    Empty,
}

/// Header text of a column (some say something else in the Recycle Bin).
pub(crate) fn header_label(id: ColumnId, is_bin: bool) -> &'static str {
    match id {
        ColumnId::Folder if is_bin => tr("Original location"),
        ColumnId::Modified if is_bin => tr("Date deleted"),
        _ => tr(id.label()),
    }
}

/// The cell of column `id` for `e`. `props` are the values read from the
/// file (`None` while they are being read, or for columns that don't need them).
pub(super) fn cell(
    id: ColumnId,
    e: &FileEntry,
    cx: &Ctx<'_>,
    props: Option<&Props>,
    v: &egui::Visuals,
) -> Cell {
    let text = |s: Option<String>| match s {
        Some(s) if !s.is_empty() => Cell::Text(s),
        _ => Cell::Empty,
    };
    let date = |t: Option<SystemTime>| text(t.map(|t| format::time(t, cx.date_format)));
    match id {
        // Drawn by the row itself (icon, name, tags).
        ColumnId::Name => Cell::Empty,
        ColumnId::Folder => text(folder(e, cx)),
        ColumnId::Git => git(e, cx, v),
        ColumnId::Modified => date(e.modified),
        ColumnId::Created => date(e.created),
        ColumnId::Type => Cell::Text(e.type_label()),
        ColumnId::Size => text(size(e, cx)),
        ColumnId::Dimensions => text(props.and_then(|p| p.dimensions).map(props::dimensions_text)),
        ColumnId::Length => text(props.and_then(|p| p.duration).map(props::duration_text)),
        ColumnId::DateTaken => date(props.and_then(|p| p.date_taken)),
        ColumnId::Authors => text(props.and_then(|p| p.authors.clone())),
        ColumnId::Version => text(props.and_then(|p| p.version.clone())),
        ColumnId::Attributes => text(
            props
                .and_then(|p| p.attributes)
                .map(props::attribute_letters),
        ),
    }
}

/// Where a deleted item was, or the folder of a search or flat view result
/// (relative to its root).
fn folder(e: &FileEntry, cx: &Ctx<'_>) -> Option<String> {
    if cx.is_bin {
        return cx
            .bin
            .get(&e.path)
            .map(|i| i.original_parent.display().to_string());
    }
    Some(super::details::folder_text(e, cx))
}

fn git(e: &FileEntry, cx: &Ctx<'_>, v: &egui::Visuals) -> Cell {
    match cx.git.and_then(|g| g.state(&e.path)) {
        Some(state) => Cell::Symbol(state.letter(), git_color(state, v)),
        None => Cell::Empty,
    }
}

fn size(e: &FileEntry, cx: &Ctx<'_>) -> Option<String> {
    match (e.is_dir, cx.sizes) {
        (false, _) => Some(format::size(e.size)),
        (true, Some(sizes)) if !e.is_symlink => Some(match sizes.get(&e.path) {
            Some(s) => format::size(s.bytes),
            None => "…".into(),
        }),
        (true, _) => None,
    }
}
