//! The Git tab of folders: the repository's recent history as a graph.

use super::*;

/// How many commits the Git tab shows.
pub(super) const MAX_COMMITS: usize = 500;

/// A repository's recent history, laid out as a graph.
pub(super) struct GitInfo {
    pub(super) root: PathBuf,
    /// e.g. "main ↑2 ↓1".
    pub(super) branch: Option<String>,
    pub(super) changed: usize,
    pub(super) commits: Vec<Commit>,
    pub(super) rows: Vec<GraphRow>,
    pub(super) lanes: usize,
    /// The repository's page on GitHub, GitLab, … (from its remote).
    pub(super) web: Option<String>,
}

/// The history of the repository `dir` is in. Kept for a few seconds, so
/// going through the folders of one repository doesn't run `git log` for
/// each of them.
pub(super) fn git_info(dir: &Path) -> Option<Arc<GitInfo>> {
    static CACHE: Mutex<Option<(Instant, Arc<GitInfo>)>> = Mutex::new(None);
    const KEEP: Duration = Duration::from_secs(10);
    let root = ox_core::git::repo_root(dir)?;
    if let Some((at, info)) = CACHE.lock().ok()?.as_ref() {
        if info.root == root && at.elapsed() < KEEP {
            return Some(info.clone());
        }
    }
    let commits = ox_core::git::log(&root, MAX_COMMITS).ok()?;
    let status = ox_core::git::status(&root).ok();
    let (rows, lanes) = ox_core::git::graph(&commits);
    let web = ox_core::git::remote_url(&root)
        .ok()
        .flatten()
        .and_then(|r| ox_core::git::web_url(&r));
    let info = Arc::new(GitInfo {
        web,
        branch: status.as_ref().and_then(|s| s.branch_label()),
        changed: status.as_ref().map_or(0, |s| s.changed_files()),
        root,
        commits,
        rows,
        lanes,
    });
    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), info.clone()));
    }
    Some(info)
}

/// Colours of the graph's lanes, repeating.
pub(super) const LANE_COLORS: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0x4c, 0x9a, 0xff),
    egui::Color32::from_rgb(0xf5, 0x9e, 0x0b),
    egui::Color32::from_rgb(0x22, 0xc5, 0x5e),
    egui::Color32::from_rgb(0xe1, 0x4d, 0x8a),
    egui::Color32::from_rgb(0x8b, 0x5c, 0xf6),
    egui::Color32::from_rgb(0x14, 0xb8, 0xa6),
    egui::Color32::from_rgb(0xef, 0x44, 0x44),
    egui::Color32::from_rgb(0xa3, 0xa3, 0x3a),
];

pub(super) fn lane_color(lane: usize) -> egui::Color32 {
    LANE_COLORS[lane % LANE_COLORS.len()]
}

/// The repository's branch and history as a commit graph: one row per
/// commit with its branches and tags, message, author and date. Clicking a
/// commit copies its hash.
pub(super) fn git_graph(ui: &mut Ui, git: &GitInfo, date_fmt: &str, actions: &mut Vec<Action>) {
    use egui::{epaint::CubicBezierShape, pos2, vec2, Sense, Stroke};
    const LANE_W: f32 = 14.0;
    const ROW_H: f32 = 38.0;
    /// More lanes than this are cut off at the right.
    const MAX_LANES: usize = 8;

    let name = git
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| git.root.display().to_string());
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("📦 {name}")).strong());
        if let Some(branch) = &git.branch {
            let chip =
                egui::Button::new(RichText::new(format!("🌿 {branch}")).color(lane_color(0)))
                    .frame(false);
            if ui.add(chip).on_hover_text(tr("Copy branch name")).clicked() {
                actions.push(Action::CopyGitBranch(git.root.clone()));
            }
        }
        if let Some(web) = &git.web {
            let host = web
                .trim_start_matches("https://")
                .split('/')
                .next()
                .unwrap_or_default();
            if ui
                .small_button(format!("🌐 {host}"))
                .on_hover_text(web.as_str())
                .clicked()
            {
                actions.push(Action::OpenGitRemote(git.root.clone()));
            }
        }
    });
    let changes = if git.changed == 0 {
        tr("No changes").to_owned()
    } else {
        ox_core::trf!(
            "{v1} changed",
            v1 = ox_core::i18n::count(git.changed as u64, "file")
        )
    };
    ui.label(RichText::new(changes).weak().small());
    if git.commits.is_empty() {
        ui.add_space(8.0);
        ui.label(RichText::new(tr("No commits yet.")).weak());
        return;
    }
    ui.add_space(4.0);

    let graph_w = git.lanes.min(MAX_LANES) as f32 * LANE_W + 6.0;
    let v = ui.visuals().clone();
    let bg = v.panel_fill;
    egui::ScrollArea::vertical()
        .id_salt("preview_git")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, git.commits.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let (c, row) = (&git.commits[i], &git.rows[i]);
                let (rect, resp) =
                    ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                let painter = ui.painter_at(rect);
                if resp.hovered() {
                    painter.rect_filled(rect, 4.0, v.widgets.hovered.weak_bg_fill);
                }
                // The graph.
                let x = |lane: usize| rect.left() + 3.0 + LANE_W * (lane as f32 + 0.5);
                let (top, mid, bottom) = (rect.top(), rect.center().y, rect.bottom());
                let graph = painter.with_clip_rect(rect.with_max_x(rect.left() + graph_w));
                let stroke = |lane: usize| Stroke::new(2.0, lane_color(lane));
                let curve = |from: egui::Pos2, to: egui::Pos2, lane: usize| {
                    if (from.x - to.x).abs() < 0.5 {
                        graph.line_segment([from, to], stroke(lane));
                    } else {
                        let h = (to.y - from.y) / 2.0;
                        graph.add(CubicBezierShape::from_points_stroke(
                            [from, pos2(from.x, from.y + h), pos2(to.x, to.y - h), to],
                            false,
                            egui::Color32::TRANSPARENT,
                            stroke(lane),
                        ));
                    }
                };
                for &l in &row.through {
                    graph.line_segment([pos2(x(l), top), pos2(x(l), bottom)], stroke(l));
                }
                for &l in &row.from_above {
                    curve(pos2(x(l), top), pos2(x(row.lane), mid), l);
                }
                for &l in &row.to_below {
                    curve(pos2(x(row.lane), mid), pos2(x(l), bottom), l);
                }
                let dot = pos2(x(row.lane), mid);
                let color = lane_color(row.lane);
                if c.parents.len() > 1 {
                    // Merges: a ring.
                    graph.circle(dot, 4.0, bg, Stroke::new(2.0, color));
                } else {
                    graph.circle(dot, 4.5, color, Stroke::new(1.5, bg));
                }

                // Branches, tags and the message, then hash, author and date.
                let text_x = rect.left() + graph_w + 4.0;
                let text_w = (rect.right() - text_x - 4.0).max(10.0);
                let mut job = egui::text::LayoutJob::default();
                job.wrap.max_width = text_w;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                let font = egui::FontId::proportional(12.5);
                for r in &c.refs {
                    // A remote branch next to its local one says nothing new.
                    if let GitRef::Remote(name) = r {
                        let local = name.split_once('/').map_or("", |(_, b)| b);
                        if c.refs
                            .iter()
                            .any(|o| matches!(o, GitRef::Head(n) | GitRef::Branch(n) if n == local))
                        {
                            continue;
                        }
                    }
                    let (label, color) = ref_style(r);
                    job.append(
                        &format!(" {label} "),
                        0.0,
                        egui::TextFormat {
                            font_id: egui::FontId::proportional(11.0),
                            color: egui::Color32::WHITE,
                            background: color.gamma_multiply(0.85),
                            ..Default::default()
                        },
                    );
                    job.append(
                        " ",
                        0.0,
                        egui::TextFormat::simple(font.clone(), v.text_color()),
                    );
                }
                job.append(
                    &c.subject,
                    0.0,
                    egui::TextFormat::simple(font, v.text_color()),
                );
                let galley = painter.layout_job(job);
                painter.galley(pos2(text_x, top + 4.0), galley, v.text_color());
                let when = commit_time(c, date_fmt);
                painter.text(
                    pos2(text_x, top + 21.0),
                    egui::Align2::LEFT_TOP,
                    format!("{} · {} · {when}", c.short(), c.author),
                    egui::FontId::proportional(11.0),
                    v.weak_text_color(),
                );

                let resp = resp.on_hover_ui(|ui| {
                    ui.set_max_width(360.0);
                    ui.label(RichText::new(&c.subject).strong());
                    if !c.refs.is_empty() {
                        let names: Vec<&str> = c.refs.iter().map(GitRef::name).collect();
                        ui.label(names.join(", "));
                    }
                    ui.label(RichText::new(&c.hash).monospace().small());
                    ui.label(format!("{} · {when}", c.author));
                    ui.label(RichText::new(tr("Click to copy the hash.")).weak().small());
                });
                if resp.clicked() {
                    ui.ctx().copy_text(c.hash.clone());
                    actions.push(Action::Toast(ox_core::trf!(
                        "Copied {what}",
                        what = c.short()
                    )));
                }
                resp.context_menu(|ui| {
                    if ui.button(tr("Copy commit hash")).clicked() {
                        ui.ctx().copy_text(c.hash.clone());
                        ui.close();
                    }
                    if ui.button(tr("Copy message")).clicked() {
                        ui.ctx().copy_text(c.subject.clone());
                        ui.close();
                    }
                });
            }
        });
    if git.commits.len() >= MAX_COMMITS {
        ui.label(
            RichText::new(ox_core::trf!(
                "The newest {n} commits are shown.",
                n = MAX_COMMITS
            ))
            .weak()
            .small(),
        );
    }
}

/// A ref's chip text and colour: the current branch, tags, remote branches
/// and local branches each look different.
pub(super) fn ref_style(r: &GitRef) -> (String, egui::Color32) {
    use egui::Color32;
    match r {
        GitRef::Head(b) => (format!("● {b}"), Color32::from_rgb(0x16, 0xa3, 0x4a)),
        GitRef::Detached => ("HEAD".into(), Color32::from_rgb(0x16, 0xa3, 0x4a)),
        GitRef::Branch(b) => (b.clone(), Color32::from_rgb(0x25, 0x63, 0xeb)),
        GitRef::Remote(b) => (b.clone(), Color32::from_rgb(0x6b, 0x72, 0x80)),
        GitRef::Tag(t) => (format!("🏷 {t}"), Color32::from_rgb(0xb4, 0x53, 0x09)),
    }
}

pub(super) fn commit_time(c: &Commit, date_fmt: &str) -> String {
    let t = SystemTime::UNIX_EPOCH + Duration::from_secs(c.time.max(0) as u64);
    format::time(t, date_fmt)
}
