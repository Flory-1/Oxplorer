//! Data files in the preview: CSV/TSV as a table, JSON/XML/YAML/TOML as a
//! tree and SQLite databases table by table. The file's text stays in the
//! Text tab. Items inside archives are read into memory (as much as the
//! views take); databases are copied to a temporary file while shown.

use super::*;
use ox_core::datatable::{self, CsvFormat, Table};
use ox_core::datatree::{self, Format, ParseError, Tree};
use ox_core::sqlite_view::{self, DbObject};
use ox_core::temp_copy::TempCopy;
use std::cell::RefCell;

/// How many rows of a database table are shown.
const DB_ROWS: usize = 1000;
/// Largest database inside an archive that is copied out to be shown.
const MAX_ARCHIVED_DB: u64 = 256 * 1024 * 1024;

/// Where a data file's bytes are.
#[derive(Clone, Copy)]
pub(super) enum Place<'a> {
    File,
    /// Inside the archive `.0`, at `.1`.
    Archive(&'a Path, &'a Path),
}

pub(super) struct Data {
    /// The file as text (a `Content::Text`), for the Text tab.
    pub text: Option<Box<Content>>,
    view: View,
    /// What the tabs remember while the file is shown.
    state: RefCell<State>,
}

/// What the background load found.
pub(super) struct LoadedData {
    text: Option<Box<Loaded>>,
    view: View,
    /// A database's first table.
    rows: Option<Result<Table, String>>,
}

enum View {
    Csv(Table, CsvFormat),
    Tree(Box<Result<Tree, ParseError>>),
    Db {
        path: PathBuf,
        objects: Result<Vec<DbObject>, String>,
        /// The temporary copy at `path` of a database inside an archive
        /// (deleted when the last user lets go of it).
        copy: Option<Arc<TempCopy>>,
    },
}

#[derive(Default)]
struct State {
    tree: tree::TreeState,
    /// The database table shown, its rows (or why they can't be read) and
    /// the load of another one.
    picked: usize,
    rows: Option<Result<Table, String>>,
    loading: Option<Receiver<Result<Table, String>>>,
}

/// Loads `entry` if it is a data file (`None` otherwise, or if it is too
/// large or not text after all: it is then previewed as usual).
pub(super) fn load(entry: &FileEntry, place: Place) -> Option<Loaded> {
    let ext = entry.extension();
    // The first `max` bytes, and one more if there are more.
    let read_start = |max: u64| match place {
        Place::File => read_start(&entry.path, max),
        Place::Archive(arc, inner) => ox_core::archive::read_entry(arc, inner, max + 1).ok(),
    };
    if matches!(ext.as_str(), "csv" | "tsv" | "tab") {
        let bytes = read_start(datatable::MAX_CSV_BYTES)?;
        let cut = bytes.len() as u64 > datatable::MAX_CSV_BYTES;
        let text =
            datatable::decode_csv(&bytes[..bytes.len().min(datatable::MAX_CSV_BYTES as usize)])?;
        let (table, format) = datatable::parse_csv(&text, ext != "csv", cut);
        return Some(loaded(bytes, View::Csv(table, format)));
    }
    if let Some(format) = Format::of(&ext) {
        if entry.size > datatree::MAX_BYTES {
            return None;
        }
        let bytes = read_start(datatree::MAX_BYTES)?;
        let text = datatable::decode_csv(&bytes)?;
        let tree = datatree::parse(format, &text);
        // Files that are only sometimes XML (.config, .manifest, …) are
        // shown as text if they aren't.
        let usual = matches!(
            ext.as_str(),
            "json" | "jsonc" | "xml" | "yaml" | "yml" | "toml"
        );
        if tree.is_err() && !usual {
            return None;
        }
        return Some(loaded(bytes, View::Tree(Box::new(tree))));
    }
    match place {
        Place::File if sqlite_view::is_database(&entry.path) => {
            Some(database_view(entry.path.clone(), None))
        }
        Place::Archive(arc, inner) if sqlite_view::has_database_extension(&entry.path) => {
            archived_database(entry, arc, inner)
        }
        _ => None,
    }
}

/// A database's tables and the rows of the first one.
fn database_view(path: PathBuf, copy: Option<Arc<TempCopy>>) -> Loaded {
    let objects = sqlite_view::objects(&path);
    let rows = match &objects {
        Ok(list) => list.first().map(|o| sqlite_view::rows(&path, o, DB_ROWS)),
        Err(_) => None,
    };
    let view = View::Db {
        path,
        objects,
        copy,
    };
    Loaded::Data(Box::new(LoadedData {
        text: None,
        view,
        rows,
    }))
}

/// A database inside an archive: copied to a temporary file (SQLite needs
/// a path), opened read-only from there and deleted when no longer shown.
fn archived_database(entry: &FileEntry, arc: &Path, inner: &Path) -> Option<Loaded> {
    if entry.size > MAX_ARCHIVED_DB {
        return Some(Loaded::Error(
            tr("The database is too large to show from inside the archive.").into(),
        ));
    }
    let bytes = ox_core::archive::read_entry(arc, inner, MAX_ARCHIVED_DB + 1).ok()?;
    if !sqlite_view::is_database_header(&bytes) {
        return None;
    }
    let name = entry.path.file_name()?.to_string_lossy();
    match TempCopy::write(&bytes, &name) {
        Ok(copy) => {
            let copy = Arc::new(copy);
            Some(database_view(copy.path().to_owned(), Some(copy)))
        }
        Err(e) => Some(Loaded::Error(e.to_string())),
    }
}

/// The first `max` bytes of a file, and one more if there are more.
fn read_start(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max + 1)
        .read_to_end(&mut buf)
        .ok()?;
    Some(buf)
}

fn loaded(mut bytes: Vec<u8>, view: View) -> Loaded {
    bytes.truncate(MAX_TEXT_BYTES + 1);
    let text = match text_from_bytes(bytes) {
        t @ Loaded::Text { .. } => Some(Box::new(t)),
        _ => None,
    };
    Loaded::Data(Box::new(LoadedData {
        text,
        view,
        rows: None,
    }))
}

/// What the preview shows for loaded data.
pub(super) fn content(d: LoadedData) -> Content {
    let LoadedData { text, view, rows } = d;
    let text = text.and_then(|t| match *t {
        Loaded::Text {
            text,
            numbers,
            truncated,
            details,
        } => Some(Box::new(Content::Text {
            text,
            numbers,
            truncated,
            details,
            markdown: false,
        })),
        _ => None,
    });
    let mut state = State {
        rows,
        ..Default::default()
    };
    if let View::Tree(tree) = &view {
        if let Ok(tree) = tree.as_ref() {
            state.tree = tree::TreeState::new(tree);
        }
    }
    Content::Data(Box::new(Data {
        text,
        view,
        state: RefCell::new(state),
    }))
}

impl Data {
    pub(super) fn tabs(&self) -> Vec<PreviewTab> {
        use PreviewTab::*;
        let mut tabs = vec![
            Info,
            match self.view {
                View::Tree(_) => Tree,
                _ => Table,
            },
        ];
        tabs.extend(self.text.as_ref().map(|_| Text));
        tabs
    }

    pub(super) fn facts(&self) -> Vec<(&'static str, String)> {
        let mut facts = Vec::new();
        match &self.view {
            View::Csv(t, f) => {
                let more = if t.truncated { "+" } else { "" };
                facts.push((tr("Rows"), format!("{}{more}", t.rows.len())));
                facts.push((tr("Columns"), t.columns.len().to_string()));
                facts.push((tr("Delimiter"), f.delimiter_label().to_owned()));
                let header = if f.header { tr("Yes") } else { tr("No") };
                facts.push((tr("Header row"), header.to_owned()));
            }
            View::Tree(tree) => match tree.as_ref() {
                Ok(t) => {
                    facts.push((tr("Format"), t.format.label().to_owned()));
                    let more = if t.truncated { "+" } else { "" };
                    facts.push((tr("Nodes"), format!("{}{more}", t.nodes.len())));
                }
                Err(e) => facts.push((tr("Error"), error_text(e))),
            },
            View::Db { objects, .. } => match objects {
                Ok(list) => {
                    let views = list.iter().filter(|o| o.view).count();
                    facts.push((tr("Format"), "SQLite".to_owned()));
                    facts.push((tr("Tables"), (list.len() - views).to_string()));
                    if views > 0 {
                        facts.push((tr("Views"), views.to_string()));
                    }
                    if let Some(o) = list.get(self.state.borrow().picked) {
                        facts.push((tr("Shown"), o.name.clone()));
                        if let Some(n) = o.rows {
                            facts.push((tr("Rows"), n.to_string()));
                        }
                        let columns: Vec<String> = o
                            .columns
                            .iter()
                            .map(|(name, decl)| format!("{name} {decl}").trim_end().to_owned())
                            .collect();
                        facts.push((tr("Columns"), columns.join("\n")));
                    }
                }
                Err(e) => facts.push((tr("Error"), e.clone())),
            },
        }
        if let Some(text) = &self.text {
            facts.extend(text.facts());
        }
        facts
    }
}

/// A parse error with where it is.
fn error_text(e: &ParseError) -> String {
    match e.line {
        0 => e.message.clone(),
        _ => ox_core::trf!(
            "Line {line}, column {column}: {message}",
            line = e.line,
            column = e.column,
            message = e.message
        ),
    }
}

/// Draws the Table, Tree and Hex tabs. Returns whether it did.
pub(super) fn show(
    ui: &mut Ui,
    hex: &mut hex::HexView,
    tab: PreviewTab,
    content: &Content,
    e: &FileEntry,
    actions: &mut Vec<Action>,
) -> bool {
    match (tab, content) {
        (PreviewTab::Hex, _) => hex::show(ui, hex, e, actions),
        (PreviewTab::Table | PreviewTab::Tree, Content::Data(d)) => {
            let state = &mut *d.state.borrow_mut();
            match &d.view {
                View::Csv(t, _) => table::show(ui, t, "csv", actions),
                View::Tree(tree) => match tree.as_ref() {
                    Ok(t) => tree::show(ui, t, &mut state.tree, actions),
                    Err(err) => {
                        ui.add_space(6.0);
                        ui.colored_label(
                            ui.visuals().error_fg_color,
                            ox_core::trf!(
                                "⚠ This isn't valid {format}.",
                                format = tree_format(e).map_or("", Format::label)
                            ),
                        );
                        ui.label(error_text(err));
                    }
                },
                View::Db {
                    path,
                    objects,
                    copy,
                } => database(ui, (path, copy), objects, state, actions),
            }
        }
        _ => return false,
    }
    true
}

fn tree_format(e: &FileEntry) -> Option<Format> {
    Format::of(&e.extension())
}

/// A database: a list of its tables and views, and the rows of the one
/// picked.
fn database(
    ui: &mut Ui,
    (path, copy): (&Path, &Option<Arc<TempCopy>>),
    objects: &Result<Vec<DbObject>, String>,
    state: &mut State,
    actions: &mut Vec<Action>,
) {
    let list = match objects {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => {
            ui.label(RichText::new(tr("The database has no tables.")).weak());
            return;
        }
        Err(e) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
            return;
        }
    };
    let label = |o: &DbObject| match (o.view, o.rows) {
        (true, _) => format!("{} · {}", o.name, tr("view")),
        (false, Some(n)) => format!("{} · {}", o.name, ox_core::i18n::count(n, "row")),
        (false, None) => o.name.clone(),
    };
    let mut picked = state.picked;
    egui::ComboBox::from_id_salt("preview_db_table")
        .width(ui.available_width().min(320.0))
        .selected_text(list.get(picked).map(label).unwrap_or_default())
        .show_ui(ui, |ui| {
            for (i, o) in list.iter().enumerate() {
                ui.selectable_value(&mut picked, i, label(o));
            }
        });
    if picked != state.picked {
        state.picked = picked;
        state.rows = None;
        let (tx, rx) = crossbeam_channel::bounded(1);
        let (path, object, repaint) = (path.to_owned(), list[picked].clone(), ui.ctx().clone());
        // The copy stays until the rows are read.
        let copy = copy.clone();
        std::thread::spawn(move || {
            let _ = tx.send(sqlite_view::rows(&path, &object, DB_ROWS));
            drop(copy);
            repaint.request_repaint();
        });
        state.loading = Some(rx);
    }
    if let Some(rows) = state.loading.as_ref().and_then(|rx| rx.try_recv().ok()) {
        state.rows = Some(rows);
        state.loading = None;
    }
    ui.add_space(4.0);
    match &state.rows {
        Some(Ok(t)) => table::show(ui, t, &list[state.picked].name, actions),
        Some(Err(e)) => {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        None => {
            ui.spinner();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn data_files_inside_archives_get_their_views() {
        let dir = tempfile::tempdir().unwrap();
        let csv = dir.path().join("people.csv");
        let json = dir.path().join("config.json");
        std::fs::write(&csv, "name,age\nAda,36\nAlan,41\n").unwrap();
        std::fs::write(&json, r#"{"a": [1, 2], "b": {"c": true}}"#).unwrap();
        let zip = dir.path().join("data.zip");
        ox_core::archive::compress_zip(&[csv, json], &zip, &AtomicBool::new(false)).unwrap();
        let items = ox_core::archive::list(&zip, Path::new("")).unwrap();
        let data = |name: &str| {
            let e = items.iter().find(|e| e.name == name).unwrap();
            let (arc, inner) = ox_core::archive::split_virtual(&e.path).unwrap();
            match load(e, Place::Archive(&arc, &inner)) {
                Some(Loaded::Data(d)) => *d,
                _ => panic!("no data view for {name}"),
            }
        };
        let d = data("people.csv");
        assert!(matches!(&d.view, View::Csv(t, _) if t.rows.len() == 2));
        assert!(d.text.is_some(), "the Text tab too");
        let d = data("config.json");
        assert!(matches!(&d.view, View::Tree(t) if t.is_ok()));
    }
}
