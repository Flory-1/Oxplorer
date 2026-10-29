use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::extract::{extract, format_of, Format};
use super::store::{Store, Text};
use super::tokens::{is_exact_word, query_words, words, MAX_RUN, MAX_WORD, STEP};
use super::*;
use crate::search::{self, Query, SearchEvent, SearchOptions};

fn set(words: &[&str]) -> HashSet<String> {
    words.iter().map(|w| w.to_string()).collect()
}

#[test]
fn words_are_lower_cased_runs_of_letters_and_digits() {
    let w = words("Hello, Wörld! TODO-list 42 ΣΊΣΥΦΟΣ", 1000).unwrap();
    assert_eq!(w, set(&["hello", "wörld", "todo", "list", "42", "σίσυφοσ"]));
    assert_eq!(query_words("  \"TODO list\" todo "), ["todo", "list"]);
    assert!(query_words("... --").is_empty());
    assert!(is_exact_word("Invoice42"));
    assert!(!is_exact_word("two words"));
    assert!(!is_exact_word("wörld"), "not ASCII");
    assert!(words("a b c d", 3).is_none(), "too many words");
    assert!(words(&"x".repeat(MAX_RUN + 1), 1000).is_none());
}

#[test]
fn long_words_are_stored_in_pieces_that_cover_every_part() {
    let long: String = (0..200)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    let stored = words(&long, 1000).unwrap();
    assert!(stored.iter().all(|w| w.chars().count() <= MAX_WORD));
    // Every part of the word, cut as queries are, is inside a stored piece.
    for start in 0..long.len() {
        for len in [1, 5, STEP, STEP + 9, 120] {
            let Some(part) = long.get(start..(start + len).min(long.len())) else {
                continue;
            };
            for q in query_words(part) {
                assert!(stored.iter().any(|w| w.contains(&q)), "{q}");
            }
        }
    }
}

#[test]
fn every_part_of_a_text_is_found_through_its_words() {
    let text = "Die Straße hieß \"Hauptstraße 12a\"; e-mail: info@example.org (TODO: fix)";
    let stored = words(text, 1000).unwrap();
    let chars: Vec<char> = text.chars().collect();
    for a in 0..chars.len() {
        for b in a + 1..=chars.len() {
            let part: String = chars[a..b].iter().collect();
            for q in query_words(&part) {
                assert!(stored.iter().any(|w| w.contains(&q)), "{part:?} → {q:?}");
            }
        }
    }
}

/// A zip file with these parts (an Office or OpenDocument file).
fn zip_file(path: &Path, parts: &[(&str, &str)]) {
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    for (name, body) in parts {
        zip.start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(body.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
}

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;

fn docx(path: &Path, paragraphs: &[&str]) {
    let body: String = paragraphs
        .iter()
        .map(|p| format!("<w:p><w:r><w:t>{p}</w:t></w:r></w:p>"))
        .collect();
    let xml =
        format!(r#"<?xml version="1.0"?><w:document {W}><w:body>{body}</w:body></w:document>"#);
    zip_file(
        path,
        &[
            ("[Content_Types].xml", "<Types/>"),
            ("word/document.xml", &xml),
            (
                "word/header1.xml",
                &format!("<w:hdr {W}><w:p><w:r><w:t>Header text</w:t></w:r></w:p></w:hdr>"),
            ),
            (
                "word/styles.xml",
                "<w:styles><w:t>not text</w:t></w:styles>",
            ),
        ],
    );
}

#[test]
fn text_of_word_excel_powerpoint_and_opendocument_files() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();

    let doc = p.join("letter.docx");
    docx(
        &doc,
        &["Dear Sir &amp; Madam,", "the annual report is late."],
    );
    let text = extract(&doc, 1 << 20).unwrap().unwrap();
    assert!(text.contains("Dear Sir & Madam,"), "{text}");
    assert!(text.contains("\nthe annual report is late."), "{text}");
    assert!(text.contains("Header text"));
    assert!(!text.contains("not text"), "styles aren't text");

    let x = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main""#;
    let xlsx = p.join("budget.xlsx");
    zip_file(
        &xlsx,
        &[
            (
                "xl/sharedStrings.xml",
                &format!(r#"<sst {x}><si><t>Rent</t></si><si><t>Coffee beans</t></si></sst>"#),
            ),
            (
                "xl/worksheets/sheet1.xml",
                &format!(
                    r#"<worksheet {x}><sheetData><row r="1"><c r="A1" t="s"><v>1</v></c><c r="B1"><v>1234.5</v></c><c r="C1" t="inlineStr"><is><t>inline note</t></is></c></row></sheetData></worksheet>"#
                ),
            ),
        ],
    );
    let text = extract(&xlsx, 1 << 20).unwrap().unwrap();
    for s in ["Rent", "Coffee beans", "1234.5", "inline note"] {
        assert!(text.contains(s), "{s} in {text:?}");
    }
    let w = words(&text, 1000).unwrap();
    assert!(!w.contains("1"), "a shared string number isn't text: {w:?}");

    let a = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main""#;
    let pptx = p.join("talk.pptx");
    zip_file(
        &pptx,
        &[
            (
                "ppt/slides/slide1.xml",
                &format!("<p:sld {a}><a:p><a:r><a:t>Quarterly results</a:t></a:r></a:p></p:sld>"),
            ),
            (
                "ppt/notesSlides/notesSlide1.xml",
                &format!("<p:notes {a}><a:p><a:r><a:t>Speak slowly</a:t></a:r></a:p></p:notes>"),
            ),
        ],
    );
    let text = extract(&pptx, 1 << 20).unwrap().unwrap();
    assert!(text.contains("Quarterly results") && text.contains("Speak slowly"));

    let odt = p.join("notes.odt");
    zip_file(
        &odt,
        &[(
            "content.xml",
            r#"<office:document-content xmlns:office="o" xmlns:text="t"><office:body><text:p>Hello<text:s/>open <text:span>document</text:span></text:p></office:body></office:document-content>"#,
        )],
    );
    let text = extract(&odt, 1 << 20).unwrap().unwrap();
    assert!(text.contains("Hello open document"), "{text:?}");

    // Not a zip: an error, not a panic.
    fs::write(p.join("broken.docx"), "not a zip").unwrap();
    assert!(extract(&p.join("broken.docx"), 1 << 20).is_err());
    // A damaged PDF has no text.
    fs::write(p.join("broken.pdf"), "%PDF-1.4 garbage").unwrap();
    assert_eq!(extract(&p.join("broken.pdf"), 1 << 20).unwrap(), None);
}

#[test]
fn text_files_in_their_encoding_and_binary_files() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    let read = |name: &str, bytes: &[u8]| {
        fs::write(p.join(name), bytes).unwrap();
        extract(&p.join(name), 1 << 20).unwrap()
    };
    assert_eq!(
        read("a.rs", b"fn main() {}").as_deref(),
        Some("fn main() {}")
    );
    let mut utf16 = vec![0xFF, 0xFE];
    utf16.extend("Grüße".encode_utf16().flat_map(u16::to_le_bytes));
    assert_eq!(read("u.txt", &utf16).as_deref(), Some("Grüße"));
    // Windows-1252: read as such next to the UTF-8 reading.
    let old = read("old.txt", b"M\xe4dchen").unwrap();
    assert!(old.contains("Mädchen"), "{old:?}");
    assert_eq!(read("data.bin", b"ab\0cd"), None);
    assert_eq!(read("photo.jpg", b"text, but a picture"), None);
    assert_eq!(format_of(Path::new("x.svg")), Format::Text);
    assert_eq!(format_of(Path::new("x.DOCX")), Format::Office);
    assert_eq!(format_of(Path::new("x.mp4")), Format::Binary);
    assert_eq!(format_of(Path::new("Makefile")), Format::Text);
}

fn put(s: &mut Store, path: &str, size: u64, text: &str) {
    let w = words(text, 1000).unwrap();
    s.put(Path::new(path), size, 1, Text::Words, w);
}

/// Paths of the files `query` gives as candidates.
fn candidates(s: &Store, query: &str) -> Vec<String> {
    let bits = s.candidates(&query_words(query));
    let mut out: Vec<String> = s
        .files()
        .filter(|(i, _)| bits.get(*i))
        .map(|(_, f)| f.path.to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn store_add_update_remove_and_narrow() {
    let mut s = Store::default();
    put(&mut s, "/a.txt", 1, "the annual report");
    put(&mut s, "/b.txt", 2, "report of the year");
    put(&mut s, "/c.txt", 3, "nothing here");
    s.put(Path::new("/d.bin"), 4, 1, Text::None, []);
    s.put(Path::new("/e.log"), 5, 1, Text::Always, []);
    assert_eq!(s.len(), 5);
    assert_eq!(candidates(&s, "report"), ["/a.txt", "/b.txt", "/e.log"]);
    // Parts of words, and all words of a phrase.
    assert_eq!(candidates(&s, "PORT"), ["/a.txt", "/b.txt", "/e.log"]);
    assert_eq!(candidates(&s, "annual report"), ["/a.txt", "/e.log"]);
    assert_eq!(candidates(&s, "missing"), ["/e.log"]);
    // Changed: the old words go.
    put(&mut s, "/a.txt", 10, "something else");
    assert_eq!(candidates(&s, "annual"), ["/e.log"]);
    assert_eq!(candidates(&s, "something"), ["/a.txt", "/e.log"]);
    assert!(s.is_current(Path::new("/a.txt"), 10, 1));
    assert!(!s.is_current(Path::new("/a.txt"), 1, 1));
    assert!(s.remove(Path::new("/b.txt")));
    assert!(!s.remove(Path::new("/b.txt")));
    assert_eq!(candidates(&s, "report"), ["/e.log"]);
    assert_eq!(s.len(), 4);
    s.compact();
    assert_eq!(s.len(), 4);
    assert_eq!(candidates(&s, "something"), ["/a.txt", "/e.log"]);
    assert_eq!(s.documents(), (3, 10 + 3 + 5));
}

#[test]
fn store_is_saved_and_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("sub").join("fulltext.bin");
    let mut s = Store::default();
    for i in 0..300 {
        put(
            &mut s,
            &format!("/f{i}.txt"),
            i,
            &format!("word{i} common w{}", i % 7),
        );
    }
    put(&mut s, "/f5.txt", 99, "replaced");
    s.remove(Path::new("/f6.txt"));
    s.roots = vec![PathBuf::from("/")];
    s.complete = true;
    s.save(&file).unwrap();
    let loaded = Store::load(&file).unwrap();
    assert_eq!(loaded.len(), 299);
    assert!(loaded.complete);
    assert_eq!(loaded.roots, [PathBuf::from("/")]);
    for q in ["common", "word12", "w3", "replaced", "word5", "word6"] {
        assert_eq!(candidates(&loaded, q), candidates(&s, q), "{q}");
    }
    assert!(loaded.is_current(Path::new("/f5.txt"), 99, 1));
    // A damaged file isn't loaded.
    let bytes = fs::read(&file).unwrap();
    fs::write(&file, &bytes[..bytes.len() / 2]).unwrap();
    assert!(Store::load(&file).is_err());
}

/// Wait until the service has finished a pass.
fn wait(svc: &FullTextService) {
    let start = Instant::now();
    while svc.state() != FullTextState::Ready || svc.last_pass().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "{:?}",
            svc.state()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Names found by a walk search for `q`, with or without the text index,
/// and whether the index was used.
fn search_names(root: &Path, q: &str, index: Option<Arc<FullTextIndex>>) -> (Vec<String>, bool) {
    let mut query = Query::parse(q, false);
    if let Some(i) = index {
        query.use_text_index(i);
    }
    let h = search::start(
        vec![root.to_path_buf()],
        query,
        SearchOptions::default(),
        || {},
    );
    let mut names = Vec::new();
    for ev in h.events.iter() {
        match ev {
            SearchEvent::Results(r) => names.extend(r.into_iter().map(|e| e.name)),
            SearchEvent::Done { .. } => break,
        }
    }
    names.sort();
    (names, h.from_text_index())
}

#[test]
fn searches_use_the_index_and_confirm_its_candidates() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(root.join("node_modules")).unwrap();
    fs::write(root.join("todo.txt"), "remember the TODO list").unwrap();
    fs::write(root.join("words.md"), "list of things to do").unwrap();
    fs::write(root.join("node_modules/x.js"), "TODO in a skipped folder").unwrap();
    docx(&root.join("report.docx"), &["The annual report", "is due."]);
    docx(&root.join("apart.docx"), &["annual", "a report"]);
    let cache = dir.path().join("cache").join("fulltext.bin");
    let mut svc = FullTextService::new(cache.clone(), || {});
    let settings = FullTextSettings {
        enabled: true,
        folders: vec![root.clone()],
        max_file_mb: 1,
    };
    svc.configure(settings.config(&["node_modules".into()]));
    wait(&svc);
    let index = svc.index().unwrap();
    assert!(index.covers(Some(&root)));
    assert!(!index.covers(Some(&dir.path().join("elsewhere"))));
    let stats = index.stats();
    assert_eq!((stats.files, stats.documents), (4, 4), "{stats:?}");
    assert!(cache.exists(), "saved after the pass");

    let with = |q: &str| search_names(&root, q, Some(index.clone()));
    // The same results as reading every file, for plain text.
    assert_eq!(
        with("content:todo"),
        (vec!["todo.txt".into(), "x.js".into()], true)
    );
    assert_eq!(
        search_names(&root, "content:todo", None).0,
        with("content:todo").0
    );
    assert_eq!(with("content:\"todo list\"").0, ["todo.txt"]);
    assert_eq!(with("content:\"list of\"").0, ["words.md"]);
    assert_eq!(with("content:emember").0, ["todo.txt"]);
    // Documents: found only with the index; phrases are confirmed.
    assert_eq!(with("content:\"annual report\"").0, ["report.docx"]);
    assert!(search_names(&root, "content:\"annual report\"", None)
        .0
        .is_empty());
    assert_eq!(with("content:annual").0, ["apart.docx", "report.docx"]);
    // A text without words can't use the index.
    assert!(!with("content:\"...\"").1);

    // Changed since the pass: read as without the index.
    fs::write(root.join("words.md"), "now with a TODO, longer than before").unwrap();
    assert_eq!(with("content:todo").0, ["todo.txt", "words.md", "x.js"]);

    // A rebuild reads everything again; turning it off deletes the file.
    svc.rebuild();
    wait(&svc);
    assert_eq!(with("content:todo").0, ["todo.txt", "words.md", "x.js"]);
    svc.configure(None);
    assert!(svc.index().is_none());
    let start = Instant::now();
    while cache.exists() {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_index_is_loaded_and_updated_on_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("docs");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("a.txt"), "alpha").unwrap();
    fs::write(root.join("b.txt"), "beta").unwrap();
    let cache = dir.path().join("fulltext.bin");
    let cfg = FullTextConfig {
        folders: vec![root.clone()],
        exclude: vec![],
        max_file_bytes: 1 << 20,
    };
    {
        let mut svc = FullTextService::new(cache.clone(), || {});
        svc.configure(Some(cfg.clone()));
        wait(&svc);
        assert_eq!(svc.index().unwrap().stats().files, 2);
    }
    fs::remove_file(root.join("b.txt")).unwrap();
    fs::write(root.join("c.txt"), "gamma").unwrap();
    let mut svc = FullTextService::new(cache, || {});
    svc.configure(Some(cfg));
    wait(&svc);
    let index = svc.index().unwrap();
    assert_eq!(index.stats().files, 2);
    let names = |q: &str| search_names(&root, q, Some(index.clone())).0;
    assert_eq!(names("content:gamma"), ["c.txt"]);
    assert!(names("content:beta").is_empty());
}

/// A one-page PDF showing `text` in Helvetica.
fn pdf_file(path: &Path, text: &str) {
    let stream = format!("BT /F1 24 Tf 72 700 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut pdf = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
    }
    let xref = pdf.len();
    pdf.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for o in offsets {
        pdf.push_str(&format!("{o:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    fs::write(path, pdf).unwrap();
}

#[test]
fn text_of_pdf_files() {
    let dir = tempfile::tempdir().unwrap();
    let pdf = dir.path().join("invoice.pdf");
    pdf_file(&pdf, "Invoice number 4711");
    let text = extract(&pdf, 1 << 20).unwrap().unwrap();
    assert!(text.contains("Invoice number 4711"), "{text:?}");
    assert_eq!(extract(&pdf, 10).unwrap(), None, "over the limit");
}

#[test]
fn the_file_index_back_end_uses_the_text_index_too() {
    use crate::index::{FileIndex, IndexConfig};
    use std::sync::atomic::{AtomicBool, AtomicU64};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    docx(&root.join("plan.docx"), &["Budget for the new office"]);
    fs::write(root.join("plan.txt"), "no budget here").unwrap();
    fs::write(root.join("other.txt"), "nothing").unwrap();
    let mut svc = FullTextService::new(dir.path().join("ft.bin"), || {});
    svc.configure(Some(FullTextConfig {
        folders: vec![root.clone()],
        exclude: vec![],
        max_file_bytes: 1 << 20,
    }));
    wait(&svc);
    let files = Arc::new(FileIndex::build(
        &IndexConfig {
            roots: vec![root.clone()],
            exclude: vec![],
            threads: 2,
        },
        &AtomicBool::new(false),
        &AtomicU64::new(0),
    ));
    let mut q = Query::parse("content:\"budget for\"", false);
    q.use_text_index(svc.index().unwrap());
    let h = search::start_indexed(files, Some(root), q, SearchOptions::default(), || {});
    let mut names = Vec::new();
    for ev in h.events.iter() {
        match ev {
            SearchEvent::Results(r) => names.extend(r.into_iter().map(|e| e.name)),
            SearchEvent::Done { .. } => break,
        }
    }
    assert_eq!(names, ["plan.docx"]);
    assert!(h.from_text_index());
}

#[test]
fn default_folders_exist_and_do_not_overlap() {
    let folders = default_folders();
    let keys: Vec<String> = folders.iter().map(|f| store::path_key(f)).collect();
    for (i, k) in keys.iter().enumerate() {
        assert!(folders[i].is_dir());
        for (j, other) in keys.iter().enumerate() {
            assert!(i == j || !service::under(k, std::slice::from_ref(other)));
        }
    }
    assert!(
        FullTextSettings::default().config(&[]).is_none(),
        "off by default"
    );
}
