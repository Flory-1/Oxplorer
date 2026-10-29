//! The Tree tab: a JSON, XML, YAML or TOML file as collapsible nodes,
//! values coloured by type. Only the visible rows are drawn.

use super::*;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontId};
use ox_core::datatree::{Format, Kind, Tree};

const ROW_H: f32 = 18.0;
/// Longest value drawn on a row (the rest is cut).
const MAX_VALUE_CHARS: usize = 300;

/// Which nodes are open, and the rows that makes visible.
#[derive(Default)]
pub(super) struct TreeState {
    open: Vec<bool>,
    rows: Vec<u32>,
}

impl TreeState {
    pub(super) fn new(tree: &Tree) -> Self {
        let open = tree.default_open();
        let rows = tree.visible(&open);
        Self { open, rows }
    }

    fn set_all(&mut self, tree: &Tree, open: bool) {
        // Collapsed, the top level stays open.
        self.open = tree.nodes.iter().map(|n| open || n.depth == 0).collect();
        self.rows = tree.visible(&self.open);
    }

    fn toggle(&mut self, tree: &Tree, i: u32) {
        if let Some(o) = self.open.get_mut(i as usize) {
            *o = !*o;
            self.rows = tree.visible(&self.open);
        }
    }
}

/// Colours of the value types (for dark and light themes).
struct Palette {
    key: Color32,
    element: Color32,
    attribute: Color32,
    string: Color32,
    number: Color32,
    literal: Color32,
    weak: Color32,
}

impl Palette {
    fn of(v: &egui::Visuals) -> Self {
        let rgb = |dark: u32, light: u32| {
            let c = if v.dark_mode { dark } else { light };
            Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
        };
        Self {
            key: v.strong_text_color(),
            element: v.hyperlink_color,
            attribute: v.warn_fg_color,
            string: rgb(0x98c379, 0x22863a),
            number: rgb(0x61afef, 0x005cc5),
            literal: rgb(0xc678dd, 0x6f42c1),
            weak: v.weak_text_color(),
        }
    }
}

pub(super) fn show(ui: &mut Ui, tree: &Tree, state: &mut TreeState, actions: &mut Vec<Action>) {
    if tree.nodes.is_empty() {
        ui.label(RichText::new(tr("The file is empty.")).weak());
        return;
    }
    if state.open.len() != tree.nodes.len() {
        *state = TreeState::new(tree);
    }
    ui.horizontal(|ui| {
        if ui.button(tr("⏷ Expand all")).clicked() {
            state.set_all(tree, true);
        }
        if ui.button(tr("⏶ Collapse all")).clicked() {
            state.set_all(tree, false);
        }
        if tree.truncated {
            ui.label(
                RichText::new(ox_core::trf!(
                    "Only the first {v1} nodes are shown.",
                    v1 = tree.nodes.len()
                ))
                .weak()
                .small(),
            );
        }
    });
    ui.add_space(2.0);
    let palette = Palette::of(ui.visuals());
    let mut toggled = None;
    egui::ScrollArea::both()
        .id_salt("preview_tree")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, state.rows.len(), |ui, range| {
            for &i in &state.rows[range] {
                if row(ui, tree, i, state.open[i as usize], &palette, actions) {
                    toggled = Some(i);
                }
            }
        });
    if let Some(i) = toggled {
        state.toggle(tree, i);
    }
}

/// One node's row. Returns whether it was clicked to open or close.
fn row(
    ui: &mut Ui,
    tree: &Tree,
    i: u32,
    open: bool,
    p: &Palette,
    actions: &mut Vec<Action>,
) -> bool {
    let node = &tree.nodes[i as usize];
    let foldable = node.is_container() && node.children > 0;
    let mut toggled = false;
    ui.horizontal(|ui| {
        ui.set_height(ROW_H);
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(node.depth as f32 * 14.0);
        let arrow = match (foldable, open) {
            (false, _) => " ",
            (true, true) => "⏷",
            (true, false) => "⏵",
        };
        let arrow = ui.add_sized(
            [14.0, ROW_H],
            egui::Label::new(RichText::new(arrow).size(11.0).color(p.weak))
                .sense(egui::Sense::click()),
        );
        let text = ui.add(
            egui::Label::new(layout(tree, i, open, p))
                .extend()
                .sense(egui::Sense::click()),
        );
        toggled = foldable && (arrow.clicked() || text.clicked());
        text.context_menu(|ui| {
            if ui.button(tr("📋 Copy value")).clicked() {
                table::copy(ui, tree.copy_text(i), actions);
                ui.close();
            }
            let path = tree.path(i);
            if !path.is_empty() && ui.button(tr("📋 Copy path")).clicked() {
                table::copy(ui, path, actions);
                ui.close();
            }
        });
    });
    toggled
}

/// The row's text: key or name, then the value or a count.
fn layout(tree: &Tree, i: u32, open: bool, p: &Palette) -> LayoutJob {
    let node = &tree.nodes[i as usize];
    let font = FontId::monospace(12.0);
    let mut job = LayoutJob::default();
    let mut add = |text: &str, color: Color32| {
        job.append(text, 0.0, TextFormat::simple(font.clone(), color));
    };
    match (&node.key, node.kind) {
        (Some(name), Kind::Element) => add(&format!("<{name}>"), p.element),
        (Some(name), Kind::Attribute) => add(&format!("@{name}"), p.attribute),
        (Some(key), _) if tree.format != Format::Xml => add(key, p.key),
        (None, Kind::Text) => {}
        (None, _) if node.parent.is_some() => add(&node.index.to_string(), p.weak),
        _ => add(tree.format.label(), p.weak),
    }
    let keyed = node.kind != Kind::Text;
    let gap = match node.kind {
        Kind::Element => " ",
        _ if keyed => ": ",
        _ => "",
    };
    let value = short(&node.value);
    match node.kind {
        Kind::Object | Kind::Array | Kind::Element if node.children > 0 || value.is_empty() => {
            // Open, the children show what's in it.
            if open && node.children > 0 {
                return job;
            }
            let count = ox_core::i18n::count(node.children as u64, "item");
            let summary = match node.kind {
                Kind::Object => format!("{{…}} {count}"),
                Kind::Array => format!("[…] {count}"),
                _ => count,
            };
            add(gap, p.weak);
            add(&summary, p.weak);
            return job;
        }
        _ => add(gap, p.weak),
    }
    match node.kind {
        Kind::String | Kind::Attribute => add(&format!("\"{value}\""), p.string),
        Kind::Number => add(&value, p.number),
        Kind::Bool => add(&value, p.literal),
        Kind::Null => add(&value, p.weak),
        Kind::Date => add(&value, p.attribute),
        _ => add(&value, p.key),
    }
    job
}

/// `s` on one line, cut when long.
fn short(s: &str) -> String {
    let mut out: String = s
        .chars()
        .take(MAX_VALUE_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if s.chars().nth(MAX_VALUE_CHARS).is_some() {
        out.push('…');
    }
    out
}
