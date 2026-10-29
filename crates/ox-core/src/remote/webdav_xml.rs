//! WebDAV `PROPFIND` requests and their `multistatus` answers.

use std::time::SystemTime;

use quick_xml::events::Event;

use super::url::{decode, file_name, normalize_path};
use super::RemoteEntry;

/// Asks for what the file view shows.
pub(super) const PROPFIND_BODY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:"><d:prop><d:resourcetype/><d:getcontentlength/><d:getlastmodified/></d:prop></d:propfind>"#;

/// One `<response>`: the item's path (decoded, normalized) and its entry.
#[derive(Debug, PartialEq)]
pub(super) struct DavItem {
    pub path: String,
    pub entry: RemoteEntry,
}

#[derive(Default)]
struct Pending {
    href: String,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
    /// Properties of the current `<propstat>` (used only if its status is 200).
    ok: bool,
    prop_dir: bool,
    prop_size: Option<u64>,
    prop_modified: Option<SystemTime>,
}

/// Parse a `207 Multi-Status` body.
pub(super) fn parse_multistatus(xml: &str) -> Result<Vec<DavItem>, String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut items = Vec::new();
    // Local names of the open elements (namespace prefixes differ by server).
    let mut stack: Vec<String> = Vec::new();
    let mut cur: Option<Pending> = None;
    let mut text = String::new();
    loop {
        let event = reader.read_event().map_err(|e| e.to_string())?;
        match event {
            Event::Start(e) => {
                let name = local(e.local_name().as_ref());
                open(&name, &mut cur);
                stack.push(name);
                text.clear();
            }
            Event::Empty(e) => {
                // Opens and closes at once (`<d:collection/>`).
                let name = local(e.local_name().as_ref());
                open(&name, &mut cur);
                close(&name, &stack, &mut cur, &mut items, "");
            }
            Event::Text(t) => text.push_str(&t.xml10_content()),
            Event::CData(t) => text.push_str(&t.into_inner()),
            Event::GeneralRef(r) => match r.resolve_char_ref() {
                Ok(Some(c)) => text.push(c),
                _ => text.push(match r.xml10_content().as_ref() {
                    "lt" => '<',
                    "gt" => '>',
                    "quot" => '"',
                    "apos" => '\'',
                    _ => '&',
                }),
            },
            Event::End(_) => {
                if let Some(name) = stack.pop() {
                    close(&name, &stack, &mut cur, &mut items, text.trim());
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(items)
}

fn local(name: &str) -> String {
    name.to_ascii_lowercase()
}

fn open(name: &str, cur: &mut Option<Pending>) {
    match name {
        "response" => *cur = Some(Pending::default()),
        "propstat" => {
            if let Some(p) = cur {
                p.ok = true;
                p.prop_dir = false;
                p.prop_size = None;
                p.prop_modified = None;
            }
        }
        _ => {}
    }
}

fn close(
    name: &str,
    stack: &[String],
    cur: &mut Option<Pending>,
    items: &mut Vec<DavItem>,
    text: &str,
) {
    let parent = stack.last().map(String::as_str).unwrap_or("");
    let Some(p) = cur else {
        return;
    };
    match name {
        "href" if parent == "response" => p.href = text.to_owned(),
        "collection" if parent == "resourcetype" => p.prop_dir = true,
        "getcontentlength" => p.prop_size = text.parse().ok(),
        "getlastmodified" => p.prop_modified = parse_http_date(text),
        // `HTTP/1.1 200 OK`; other statuses (404 for unknown properties) are ignored.
        "status" if parent == "propstat" => p.ok = text.split_whitespace().nth(1) == Some("200"),
        "propstat" if p.ok => {
            p.is_dir |= p.prop_dir;
            if let Some(s) = p.prop_size {
                p.size = s;
            }
            if p.prop_modified.is_some() {
                p.modified = p.prop_modified;
            }
        }
        "response" => {
            let p = cur.take().unwrap_or_default();
            let path = href_path(&p.href);
            let name = file_name(&path).to_owned();
            let mut entry = if p.is_dir {
                RemoteEntry::dir(&name, p.modified)
            } else {
                RemoteEntry::file(&name, p.size, p.modified)
            };
            entry.name = name;
            items.push(DavItem { path, entry });
        }
        _ => {}
    }
}

/// The decoded path of an `href` (`https://host/a%20b/` or `/a%20b/`).
pub(super) fn href_path(href: &str) -> String {
    let path = match href.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("/", |i| &rest[i..]),
        None => href,
    };
    let path = normalize_path(&decode(path));
    if path.is_empty() {
        "/".into()
    } else {
        path
    }
}

/// `Tue, 15 Nov 1994 08:12:31 GMT`.
fn parse_http_date(text: &str) -> Option<SystemTime> {
    chrono::DateTime::parse_from_rfc2822(text)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc).into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEXTCLOUD: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:s="http://sabredav.org/ns" xmlns:oc="http://owncloud.org/ns">
 <d:response>
  <d:href>/remote.php/dav/files/me/Documents/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype>
   <d:getlastmodified>Mon, 01 Jul 2024 10:00:00 GMT</d:getlastmodified></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
  <d:propstat><d:prop><d:getcontentlength/></d:prop><d:status>HTTP/1.1 404 Not Found</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/me/Documents/Report%202024%20%26%20more.pdf</d:href>
  <d:propstat><d:prop><d:resourcetype/><d:getcontentlength>48213</d:getcontentlength>
   <d:getlastmodified>Tue, 02 Jul 2024 08:30:15 GMT</d:getlastmodified></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
 <d:response>
  <d:href>/remote.php/dav/files/me/Documents/Photos/</d:href>
  <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop>
   <d:status>HTTP/1.1 200 OK</d:status></d:propstat>
 </d:response>
</d:multistatus>"#;

    #[test]
    fn parses_a_nextcloud_listing() {
        let items = parse_multistatus(NEXTCLOUD).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].path, "/remote.php/dav/files/me/Documents");
        assert!(items[0].entry.is_dir);
        assert!(items[0].entry.modified.is_some());
        let pdf = &items[1].entry;
        assert_eq!(pdf.name, "Report 2024 & more.pdf");
        assert_eq!(pdf.size, 48213);
        assert!(!pdf.is_dir);
        let when = chrono::DateTime::parse_from_rfc3339("2024-07-02T08:30:15Z").unwrap();
        assert_eq!(pdf.modified, Some(when.with_timezone(&chrono::Utc).into()));
        assert!(items[2].entry.is_dir);
        assert_eq!(items[2].entry.name, "Photos");
    }

    #[test]
    fn other_prefixes_absolute_hrefs_and_entities() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:"><D:response><D:href>https://dav.example.com/share/a&amp;b.txt</D:href>
<D:propstat><D:status>HTTP/1.1 200 OK</D:status><D:prop><lp1:getcontentlength xmlns:lp1="DAV:">7</lp1:getcontentlength>
<D:resourcetype/></D:prop></D:propstat></D:response></D:multistatus>"#;
        let items = parse_multistatus(xml).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].path, "/share/a&b.txt");
        assert_eq!(items[0].entry.name, "a&b.txt");
        assert_eq!(items[0].entry.size, 7);
    }

    #[test]
    fn hrefs() {
        assert_eq!(href_path("/a%20b/"), "/a b");
        assert_eq!(href_path("http://h:8080"), "/");
        assert_eq!(href_path("http://h/x/y/"), "/x/y");
    }

    #[test]
    fn broken_xml_is_an_error() {
        assert!(parse_multistatus("<d:multistatus><d:response>").is_ok());
        assert!(parse_multistatus("<a></b>").is_err());
    }
}
