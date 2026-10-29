//! Where links point, for tooltips and the status bar.

use std::path::Path;
use std::time::{Duration, Instant};

use egui::{Context, Id};
use ox_core::entry::FileEntry;
use ox_core::links::{self, LinkInfo};

/// The link info of `path`, looked up at most every two seconds (it's
/// asked for every frame while the item is hovered or selected).
pub fn info(ctx: &Context, path: &Path) -> Option<LinkInfo> {
    let id = Id::unique(("link_info", path));
    let cached = ctx.data(|d| d.get_temp::<(Instant, Option<LinkInfo>)>(id));
    if let Some((at, info)) = cached {
        if at.elapsed() < Duration::from_secs(2) {
            return info;
        }
    }
    let info = links::info(path);
    ctx.data_mut(|d| d.insert_temp(id, (Instant::now(), info.clone())));
    info
}

/// "Junction → D:\Projects", or `None` for items that aren't links.
pub fn describe(ctx: &Context, e: &FileEntry) -> Option<String> {
    if !e.is_symlink {
        return None;
    }
    let info = info(ctx, &e.path)?;
    let target = info.resolved(&e.path);
    let mut text = format!("{} → {}", info.kind.label(), target.display());
    if !ox_core::fsops::exists(&target) {
        text.push_str(ox_core::i18n::t(" (missing)"));
    }
    Some(text)
}
