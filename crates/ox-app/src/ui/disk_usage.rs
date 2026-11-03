//! Disk usage view: a treemap of what fills a folder or drive, and a list
//! of its largest items. Clicking a folder zooms into it; that is a place of
//! its own, so Back, Up and the breadcrumbs zoom out again.

use std::path::{Path, PathBuf};

use egui::ecolor::Hsva;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Ui};
use ox_core::disk_usage::{
    self as du, LayoutOptions, NodeId, NodeKind, Scan, Tile, TileItem, UsageTree, ROOT,
};
use ox_core::entry::{display_name, FileKind};
use ox_core::format;
use ox_core::i18n::{count, t as tr};
use ox_core::settings::DiskUsageOpen;

use crate::app::{Action, DiskUsageAction, OxApp};
use crate::icons::{self, Icon};
use crate::tabs::Location;

/// Width of the list of largest items.
const LIST_WIDTH: f32 = 340.0;
const ROW_HEIGHT: f32 = 38.0;

/// Where the view is shown: zooming in a tab goes to a place of its own
/// (Back and Up zoom out), in the window or panel it changes what they show.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Host {
    Tab,
    Window,
    Panel,
}

impl Host {
    fn zoom(self, path: PathBuf) -> Action {
        match self {
            Host::Tab => Action::Navigate(Location::DiskUsage(path)),
            _ => Action::DiskUsage(DiskUsageAction::Zoom(path)),
        }
    }
}

/// The page of a tab.
pub fn show(app: &mut OxApp, ui: &mut Ui) {
    let Location::DiskUsage(path) = app.tabs.active().location().clone() else {
        return;
    };
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| body(app, ui, &path, Host::Tab));
}

/// The window, when disk usage opens in one.
pub fn window(app: &mut OxApp, ctx: &egui::Context) {
    let Some(path) = app.disk_usage.panel.clone() else {
        return;
    };
    if app.settings.browsing.disk_usage_opens != DiskUsageOpen::Window {
        return;
    }
    let screen = ctx.content_rect();
    let mut open = true;
    egui::Window::new(ox_core::trf!(
        "🖴 Disk usage: {name}",
        name = folder_name(&path)
    ))
    .id(egui::Id::unique("disk_usage_window"))
    .open(&mut open)
    .collapsible(false)
    .resizable(true)
    .default_size(vec2(
        (screen.width() * 0.75).min(1100.0),
        (screen.height() * 0.7).min(720.0),
    ))
    .min_size(vec2(360.0, 260.0))
    .show(ctx, |ui| body(app, ui, &path, Host::Window));
    if !open {
        app.push(Action::DiskUsage(DiskUsageAction::Close));
    }
}

/// Whether disk usage is shown at the right, in the preview's place.
pub fn in_panel(app: &OxApp) -> bool {
    app.disk_usage.panel.is_some()
        && app.settings.browsing.disk_usage_opens == DiskUsageOpen::Preview
}

/// The panel at the right, when disk usage opens in the preview's place.
pub fn panel(app: &mut OxApp, ui: &mut Ui) {
    let Some(path) = app.disk_usage.panel.clone() else {
        return;
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr("🖴 Disk usage")).strong().size(15.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("✖").on_hover_text(tr("Close")).clicked() {
                app.push(Action::DiskUsage(DiskUsageAction::Close));
            }
        });
    });
    ui.separator();
    body(app, ui, &path, Host::Panel);
}

fn folder_name(path: &Path) -> String {
    match path.file_name() {
        Some(_) => display_name(path),
        None => path.display().to_string(),
    }
}

fn body(app: &mut OxApp, ui: &mut Ui, path: &Path, host: Host) {
    // Space used on the drive, to show how far the scan of a drive has got.
    let used = app
        .drives()
        .iter()
        .find(|d| d.mount == path && d.remote.is_none() && d.total > 0)
        .map(|d| d.total.saturating_sub(d.available));
    let colors = Colors::new(crate::theme::accent(&app.settings), ui.visuals().dark_mode);
    let mut actions = Vec::new();
    match app.disk_usage.for_path(path) {
        Some(scan) => view(ui, scan, path, used, &colors, host, &mut actions),
        // The scan starts on the next frame.
        None => {
            ui.spinner();
        }
    }
    app.actions.extend(actions);
}

/// Below this width the list goes under the map instead of beside it.
const NARROW: f32 = 620.0;

fn view(
    ui: &mut Ui,
    scan: &Scan,
    path: &Path,
    used: Option<u64>,
    colors: &Colors,
    host: Host,
    actions: &mut Vec<Action>,
) {
    let narrow = ui.available_width() < NARROW;
    top_bar(ui, scan, path, host, narrow, actions);
    if let (true, Some(used)) = (scan.is_running(), used) {
        let scanned = scan.tree().node(ROOT).bytes;
        let frac = scanned as f32 / used.max(1) as f32;
        ui.add(egui::ProgressBar::new(frac.min(1.0)).desired_height(6.0));
    }
    ui.add_space(6.0);
    let tree = scan.tree();
    let Some(id) = tree.find(path) else {
        ui.label(RichText::new(tr("Not counted yet…")).weak());
        return;
    };
    if tree.node(id).bytes == 0 {
        let text = if scan.is_running() {
            tr("Counting…")
        } else {
            tr("There is nothing in this folder that takes up space.")
        };
        ui.label(RichText::new(text).weak());
        return;
    }
    let full = ui.available_rect_before_wrap();
    let (map, list) = if narrow {
        let map_h = (full.height() * 0.55).max(120.0).min(full.height());
        (
            Rect::from_min_size(full.min, vec2(full.width(), map_h)),
            Rect::from_min_max(pos2(full.min.x, full.min.y + map_h + 10.0), full.max),
        )
    } else {
        let list_w = LIST_WIDTH.min(full.width() * 0.4);
        (
            Rect::from_min_max(full.min, pos2(full.max.x - list_w - 10.0, full.max.y)),
            Rect::from_min_max(pos2(full.max.x - list_w, full.min.y), full.max),
        )
    };
    treemap(ui, map, &tree, id, colors, host, actions);
    if list.height() > ROW_HEIGHT {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(list));
        largest(&mut child, &tree, id, colors, host, actions);
    }
    ui.advance_cursor_after_rect(full);
}

/// Breadcrumbs from the scanned folder to the shown one, how the scan is
/// going, and Stop / Scan again (in a narrow place, on a row of their own).
fn top_bar(
    ui: &mut Ui,
    scan: &Scan,
    path: &Path,
    host: Host,
    narrow: bool,
    actions: &mut Vec<Action>,
) {
    let crumbs = |ui: &mut Ui, actions: &mut Vec<Action>| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let mut parts: Vec<&Path> = path
            .ancestors()
            .take_while(|p| p.starts_with(scan.root()))
            .collect();
        parts.reverse();
        for (i, p) in parts.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("⏵").weak());
            }
            let last = i + 1 == parts.len();
            let text = RichText::new(folder_name(p)).size(14.0);
            let text = if last { text.strong() } else { text };
            if ui.add(egui::Button::new(text).frame(false)).clicked() && !last {
                actions.push(host.zoom(p.to_path_buf()));
            }
        }
    };
    if narrow {
        ui.horizontal_wrapped(|ui| crumbs(ui, actions));
        ui.horizontal_wrapped(|ui| scan_status(ui, scan, path, host, actions));
    } else {
        ui.horizontal(|ui| {
            crumbs(ui, actions);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                scan_status(ui, scan, path, host, actions);
            });
        });
    }
}

/// The buttons and how far the scan is (laid out right to left when wide).
fn scan_status(ui: &mut Ui, scan: &Scan, path: &Path, host: Host, actions: &mut Vec<Action>) {
    let (bytes, files) = {
        let tree = scan.tree();
        let root = tree.node(ROOT);
        (root.bytes, root.files)
    };
    ui.spacing_mut().item_spacing.x = 8.0;
    if host != Host::Tab {
        if ui.button("📑").on_hover_text(tr("Open in a tab")).clicked() {
            actions.push(Action::DiskUsage(DiskUsageAction::ToTab));
        }
        if let Some(parent) = path.parent() {
            if ui.button("⬆").on_hover_text(tr("Up")).clicked() {
                actions.push(host.zoom(parent.to_path_buf()));
            }
        }
    }
    let target = path.to_path_buf();
    if scan.is_running() {
        if ui.button(tr("⏹ Stop")).clicked() {
            actions.push(Action::DiskUsage(DiskUsageAction::Stop(target)));
        }
        ui.spinner();
    } else if ui.button(tr("🔄 Scan again")).clicked() {
        actions.push(Action::DiskUsage(DiskUsageAction::Rescan(target)));
    }
    let (files, size) = (count(files, "file"), format::size(bytes));
    let status = if scan.is_running() {
        ox_core::trf!("Scanning… {files}, {size}", files = files, size = size)
    } else if scan.is_done() {
        ox_core::trf!(
            "{files}, {size}, counted in {time}",
            files = files,
            size = size,
            time = format::duration(scan.elapsed())
        )
    } else {
        ox_core::trf!(
            "Stopped: {files}, {size} so far",
            files = files,
            size = size
        )
    };
    ui.label(RichText::new(status).weak());
    let unreadable = scan.unreadable();
    if unreadable > 0 {
        ui.label(RichText::new(ox_core::trf!(
            "{folders} couldn't be read",
            folders = count(unreadable, "folder")
        )))
        .on_hover_text(tr("Their sizes are missing (no permission, or in use)."));
    }
}

/// Fill colours: one hue per item of the shown folder (from the accent
/// colour on), lighter or darker for the levels inside it.
struct Colors {
    base: f32,
    dark: bool,
}

impl Colors {
    fn new(accent: Color32, dark: bool) -> Self {
        Self {
            base: Hsva::from(accent).h,
            dark,
        }
    }

    fn tile(&self, top: usize, depth: u8, kind: Option<NodeKind>) -> Color32 {
        // Steps of the golden angle keep neighbouring hues apart.
        let h = (self.base + top as f32 * 0.381_966).fract();
        let d = f32::from(depth);
        let (mut s, v) = if self.dark {
            (0.5, 0.5 + 0.08 * d)
        } else {
            (0.42, 0.9 - 0.07 * d)
        };
        match kind {
            Some(NodeKind::Folder) => {}
            Some(NodeKind::File) => s *= 0.6,
            // Small files and "Other": grey.
            _ => s = 0.0,
        }
        Hsva::new(h, s, v.clamp(0.2, 0.95), 1.0).into()
    }

    fn of(&self, tree: &UsageTree, t: &Tile) -> Color32 {
        let kind = match t.item {
            TileItem::Node(n) => Some(tree.node(n).kind),
            TileItem::Other { .. } => None,
        };
        self.tile(t.top, t.depth, kind)
    }
}

/// Black or white, whichever reads better on `fill`.
fn text_on(fill: Color32) -> Color32 {
    let [r, g, b, _] = fill.to_array();
    let luma = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b);
    if luma > 150.0 {
        Color32::from_gray(20)
    } else {
        Color32::from_gray(240)
    }
}

fn to_rect(r: du::Rect) -> Rect {
    Rect::from_min_size(pos2(r.x, r.y), vec2(r.w, r.h))
}

fn tile_name(tree: &UsageTree, item: TileItem) -> String {
    match item {
        TileItem::Node(n) => node_name(tree, n),
        TileItem::Other { count, .. } => ox_core::trf!("Other ({n})", n = count),
    }
}

fn node_name(tree: &UsageTree, n: NodeId) -> String {
    let node = tree.node(n);
    match node.kind {
        NodeKind::SmallFiles => ox_core::trf!("Smaller files ({n})", n = node.files),
        _ => node.name.to_string(),
    }
}

fn treemap(
    ui: &mut Ui,
    rect: Rect,
    tree: &UsageTree,
    id: NodeId,
    colors: &Colors,
    host: Host,
    actions: &mut Vec<Action>,
) {
    let resp = ui.interact(rect, ui.scope_id().with("disk-usage-map"), Sense::click());
    let opts = LayoutOptions::default();
    let area = du::Rect::new(rect.min.x, rect.min.y, rect.width(), rect.height());
    let tiles = tree.layout(id, area, &opts);
    let painter = ui.painter_at(rect);
    let gap = ui.visuals().panel_fill;
    for t in &tiles {
        let r = to_rect(t.rect);
        let fill = colors.of(tree, t);
        painter.rect_filled(r, CornerRadius::same(2), fill);
        painter.rect_stroke(
            r,
            CornerRadius::same(2),
            Stroke::new(1.0, gap),
            egui::StrokeKind::Inside,
        );
        tile_label(&painter, tree, t, r, fill, opts.header);
    }
    let hovered = resp
        .hover_pos()
        .and_then(|p| tiles.iter().rev().find(|t| t.rect.contains(p.x, p.y)))
        .copied();
    if let Some(t) = &hovered {
        painter.rect_stroke(
            to_rect(t.rect),
            CornerRadius::same(2),
            Stroke::new(2.0, ui.visuals().strong_text_color()),
            egui::StrokeKind::Inside,
        );
    }

    // A click zooms into the folder under the pointer (the one a file is in).
    if resp.clicked() {
        let folder = hovered.map(|t| match t.item {
            TileItem::Node(n) if tree.node(n).kind == NodeKind::Folder => n,
            TileItem::Node(n) => tree.node(n).parent,
            TileItem::Other { parent, .. } => parent,
        });
        if let Some(f) = folder.filter(|&f| f != id) {
            actions.push(host.zoom(tree.path(f)));
        }
    }
    if resp.double_clicked() {
        if let Some(TileItem::Node(n)) = hovered.map(|t| t.item) {
            if tree.node(n).kind == NodeKind::File {
                actions.push(Action::Open(tree.path(n)));
            }
        }
    }
    // The context menu stays with the item it was opened on.
    let menu_id = ui.scope_id().with("disk-usage-menu");
    if resp.secondary_clicked() {
        let target = match hovered.map(|t| t.item) {
            Some(TileItem::Node(n)) => Some(n),
            _ => None,
        };
        ui.data_mut(|d| d.insert_temp(menu_id, target));
    }
    resp.context_menu(
        |ui| match ui.data(|d| d.get_temp::<Option<NodeId>>(menu_id)).flatten() {
            Some(n) => item_menu(ui, tree, n, host, actions),
            None => ui.close(),
        },
    );
    if let Some(t) = hovered {
        let total = tree.node(id).bytes;
        resp.on_hover_ui_at_pointer(|ui| tooltip(ui, tree, &t, total, id));
    }
}

/// The name (and the size, if there's room) on a tile; on a folder with its
/// items inside, in the strip at its top.
fn tile_label(
    painter: &egui::Painter,
    tree: &UsageTree,
    t: &Tile,
    r: Rect,
    fill: Color32,
    header: f32,
) {
    if r.width() < 36.0 || r.height() < 14.0 {
        return;
    }
    let bytes = match t.item {
        TileItem::Node(n) => tree.node(n).bytes,
        TileItem::Other { bytes, .. } => bytes,
    };
    let color = text_on(fill);
    let clip = painter.with_clip_rect(r.shrink(2.0).intersect(painter.clip_rect()));
    let name = tile_name(tree, t.item);
    let font = FontId::proportional(12.0);
    if t.nested {
        let text = format!("{name}  {}", format::size(bytes));
        clip.text(
            pos2(r.left() + 4.0, r.top() + header / 2.0),
            Align2::LEFT_CENTER,
            text,
            font,
            color,
        );
        return;
    }
    clip.text(
        pos2(r.left() + 4.0, r.top() + 3.0),
        Align2::LEFT_TOP,
        name,
        font,
        color,
    );
    if r.height() >= 32.0 {
        clip.text(
            pos2(r.left() + 4.0, r.top() + 17.0),
            Align2::LEFT_TOP,
            format::size(bytes),
            FontId::proportional(11.0),
            color.gamma_multiply(0.85),
        );
    }
}

fn percent(part: u64, total: u64) -> String {
    format!(
        "{} %",
        format::decimal(part as f64 * 100.0 / total.max(1) as f64, 1)
    )
}

fn tooltip(ui: &mut Ui, tree: &UsageTree, t: &Tile, total: u64, shown: NodeId) {
    let shown_name = display_name(&tree.path(shown));
    match t.item {
        TileItem::Node(n) => {
            let node = tree.node(n);
            ui.label(RichText::new(tree.path(n).display().to_string()).strong());
            if node.kind == NodeKind::SmallFiles {
                ui.label(node_name(tree, n));
            }
            ui.label(ox_core::trf!(
                "{size} ({share} of {folder})",
                size = format::size(node.bytes),
                share = percent(node.bytes, total),
                folder = shown_name
            ));
            if node.kind == NodeKind::Folder {
                ui.label(RichText::new(count(node.files, "file")).weak());
                if !node.done {
                    ui.label(RichText::new(tr("Still counting…")).weak());
                }
            }
        }
        TileItem::Other {
            bytes, count: n, ..
        } => {
            ui.label(RichText::new(ox_core::trf!(
                "{items}, each too small to show",
                items = count(n as u64, "item")
            )));
            ui.label(ox_core::trf!(
                "{size} ({share} of {folder})",
                size = format::size(bytes),
                share = percent(bytes, total),
                folder = shown_name
            ));
        }
    }
}

/// Context menu of an item in the treemap or the list.
fn item_menu(ui: &mut Ui, tree: &UsageTree, n: NodeId, host: Host, actions: &mut Vec<Action>) {
    let node = tree.node(n);
    let path = tree.path(n);
    match node.kind {
        NodeKind::Folder => {
            if ui.button(tr("🖴 Zoom in")).clicked() {
                actions.push(host.zoom(path.clone()));
            }
            if ui.button(tr("📂 Open")).clicked() {
                actions.push(Action::Navigate(Location::Folder(path.clone())));
            }
            if ui.button(tr("📑 Open in new tab")).clicked() {
                actions.push(Action::OpenInNewTab(Location::Folder(path.clone()), false));
            }
        }
        NodeKind::File => {
            if ui.button(tr("📂 Open")).clicked() {
                actions.push(Action::Open(path.clone()));
            }
        }
        // The folder's small files: only the folder can be opened.
        NodeKind::SmallFiles => {
            if ui.button(tr("📂 Open folder")).clicked() {
                actions.push(Action::Navigate(Location::Folder(path)));
            }
            return;
        }
    }
    if ui.button(tr("📁 Open file location")).clicked() {
        actions.push(Action::ShowInFolder(path.clone()));
    }
    if ui.button(tr("🔗 Copy path")).clicked() {
        actions.push(Action::CopyPathText(vec![path.clone()]));
    }
    ui.separator();
    if ui.button(tr("🗑 Delete")).clicked() {
        actions.push(Action::Delete(vec![path.clone()], false));
    }
    if ui.button(tr("⛔ Delete permanently")).clicked() {
        actions.push(Action::Delete(vec![path.clone()], true));
    }
    ui.separator();
    if ui.button(tr("ℹ Properties")).clicked() {
        actions.push(Action::Properties(vec![path]));
    }
}

/// The items of the shown folder, largest first, with their share.
fn largest(
    ui: &mut Ui,
    tree: &UsageTree,
    id: NodeId,
    colors: &Colors,
    host: Host,
    actions: &mut Vec<Action>,
) {
    ui.label(RichText::new(tr("Largest items")).size(14.0).strong());
    ui.add_space(4.0);
    let kids: Vec<NodeId> = tree
        .largest_children(id)
        .into_iter()
        .filter(|&k| tree.node(k).bytes > 0)
        .collect();
    let total = tree.node(id).bytes;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_HEIGHT, kids.len(), |ui, range| {
            for i in range {
                row(ui, tree, kids[i], i, total, colors, host, actions);
            }
        });
}

#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut Ui,
    tree: &UsageTree,
    n: NodeId,
    index: usize,
    total: u64,
    colors: &Colors,
    host: Host,
    actions: &mut Vec<Action>,
) {
    let node = tree.node(n);
    let (rect, resp) =
        ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
    let v = ui.visuals();
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(6), v.widgets.hovered.weak_bg_fill);
    }
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let icon = match node.kind {
        NodeKind::Folder => Icon::Folder,
        _ => {
            let ext = Path::new(&*node.name)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            Icon::File(FileKind::from_extension(&ext))
        }
    };
    let top = rect.top() + 12.0;
    icons::paint_icon(
        &painter,
        icons::icon_rect(rect.left() + 6.0, top, 16.0),
        icon,
        v.text_color(),
        v.hyperlink_color,
    );
    let size = format::size(node.bytes);
    let size_w = 70.0;
    painter
        .with_clip_rect(Rect::from_min_max(
            rect.min,
            pos2(rect.right() - size_w, rect.bottom()),
        ))
        .text(
            pos2(rect.left() + 28.0, top),
            Align2::LEFT_CENTER,
            node_name(tree, n),
            FontId::proportional(13.0),
            v.text_color(),
        );
    painter.text(
        pos2(rect.right() - 6.0, top),
        Align2::RIGHT_CENTER,
        size,
        FontId::proportional(12.5),
        v.strong_text_color(),
    );
    // The share as a bar, in the item's colour in the treemap.
    let bar = Rect::from_min_size(
        pos2(rect.left() + 28.0, rect.bottom() - 11.0),
        vec2((rect.width() - 28.0 - size_w).max(10.0), 5.0),
    );
    painter.rect_filled(bar, CornerRadius::same(3), v.widgets.inactive.bg_fill);
    let mut part = bar;
    part.set_width(bar.width() * (node.bytes as f32 / total.max(1) as f32).clamp(0.0, 1.0));
    painter.rect_filled(
        part,
        CornerRadius::same(3),
        colors.tile(index, 0, Some(node.kind)),
    );
    painter.text(
        pos2(rect.right() - 6.0, bar.center().y),
        Align2::RIGHT_CENTER,
        percent(node.bytes, total),
        FontId::proportional(11.0),
        v.weak_text_color(),
    );

    if resp.clicked() && node.kind == NodeKind::Folder {
        actions.push(host.zoom(tree.path(n)));
    }
    if resp.double_clicked() && node.kind == NodeKind::File {
        actions.push(Action::Open(tree.path(n)));
    }
    resp.context_menu(|ui| item_menu(ui, tree, n, host, actions));
    resp.on_hover_text(tree.path(n).display().to_string());
}
