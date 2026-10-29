//! Text of Office Open XML (docx, xlsx, pptx) and OpenDocument (odt, ods,
//! odp) files: both are zip files with the text in XML parts.

use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::Path;

use quick_xml::events::Event;

/// The parts of an Office file that hold its text.
fn ooxml_part(name: &str) -> bool {
    let xml = name.ends_with(".xml");
    let word = name == "word/document.xml"
        || ["header", "footer", "footnotes", "endnotes", "comments"]
            .iter()
            .any(|p| name.starts_with(&format!("word/{p}")));
    let excel = name == "xl/sharedStrings.xml" || name.starts_with("xl/worksheets/sheet");
    let powerpoint = name.starts_with("ppt/slides/slide") || name.starts_with("ppt/notesSlides/");
    xml && (word || excel || powerpoint)
}

/// docx, xlsx, pptx: the text runs (`w:t`, `a:t`), the shared strings and
/// the cell values (but not the numbers that point into the shared strings).
pub(super) fn ooxml(path: &Path, limit: u64) -> io::Result<String> {
    zipped(path, limit, ooxml_part, Mode::Office)
}

/// odt, ods, odp: all text of `content.xml`.
pub(super) fn odf(path: &Path, limit: u64) -> io::Result<String> {
    zipped(path, limit, |n| n == "content.xml", Mode::OpenDocument)
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Office,
    OpenDocument,
}

/// The text of the parts of a zip file that `wanted` picks, reading at most
/// `limit` bytes of XML in all (zip bombs stop there).
fn zipped(
    path: &Path,
    limit: u64,
    wanted: impl Fn(&str) -> bool,
    mode: Mode,
) -> io::Result<String> {
    let mut zip = zip::ZipArchive::new(BufReader::new(File::open(path)?))
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let mut names: Vec<String> = zip
        .file_names()
        .flatten()
        .filter(|n| wanted(n))
        .map(String::from)
        .collect();
    names.sort_by(|a, b| crate::natural::natural_cmp(a, b));
    let mut left = limit.saturating_mul(4);
    let mut out = String::new();
    let mut xml = Vec::new();
    for name in names {
        let Ok(part) = zip.by_name(&name) else {
            continue;
        };
        xml.clear();
        part.take(left).read_to_end(&mut xml)?;
        left = left.saturating_sub(xml.len() as u64);
        xml_text(&xml, mode, &mut out);
        if left == 0 {
            break;
        }
    }
    Ok(out)
}

/// Append the text of one XML part to `out`, a line per paragraph.
fn xml_text(xml: &[u8], mode: Mode, out: &mut String) {
    let xml = String::from_utf8_lossy(xml);
    let mut reader = quick_xml::Reader::from_str(&xml);
    // Office: inside a text element (`t`, or a cell's value `v`).
    let mut in_text = false;
    // Office: the current cell's value is a number into the shared strings.
    let mut shared_cell = false;
    loop {
        let event = match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(e) => e,
        };
        let keep = mode == Mode::OpenDocument || in_text;
        match event {
            Event::Start(e) => match e.local_name().as_ref() {
                "t" => in_text = true,
                "v" => in_text = !shared_cell,
                "c" => {
                    shared_cell = e
                        .attributes()
                        .flatten()
                        .any(|a| a.key.local_name().as_ref() == "t" && a.value.as_ref() == "s");
                }
                _ => {}
            },
            Event::Empty(e) => match e.local_name().as_ref() {
                "tab" | "s" => out.push(' '),
                "br" | "line-break" => out.push('\n'),
                _ => {}
            },
            Event::End(e) => match e.local_name().as_ref() {
                // Runs of one paragraph follow each other directly.
                "t" | "v" => in_text = false,
                "p" | "h" | "si" | "row" => out.push('\n'),
                "c" | "table-cell" => out.push('\t'),
                _ => {}
            },
            Event::Text(t) if keep => out.push_str(&t.xml10_content()),
            Event::CData(t) if keep => out.push_str(&t),
            Event::GeneralRef(r) if keep => match r.resolve_char_ref() {
                Ok(Some(c)) => out.push(c),
                _ => out.push(match r.xml10_content().as_ref() {
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => '&',
                }),
            },
            _ => {}
        }
    }
}
