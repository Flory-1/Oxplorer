use super::*;
use std::collections::HashMap;

/// A registry made of `key|name` → value.
#[derive(Default)]
struct Map(HashMap<String, String>);

impl Map {
    fn set(mut self, key: &str, name: &str, value: &str) -> Self {
        self.0
            .insert(format!("{}|{name}", key.to_lowercase()), value.into());
        self
    }
    /// A handler class registered as a DLL (with prevhost's AppID).
    fn class(self, clsid: &str, appid: bool) -> Self {
        let key = format!(r"CLSID\{clsid}");
        let map = self.set(&format!(r"{key}\InprocServer32"), "", r"C:\h.dll");
        match appid {
            true => map.set(&key, "AppID", "{6d2b5079-2f0b-48dd-ab7f-97cec514d30b}"),
            false => map,
        }
    }
}

impl Classes for Map {
    fn value(&self, key: &str, name: &str) -> Option<String> {
        self.0
            .get(&format!("{}|{name}", key.to_lowercase()))
            .cloned()
    }
}

const A: &str = "{84f66100-ff7c-4fb4-b0c0-02cd7fb668fe}";
const B: &str = "{21e17c2f-ad6f-4b1e-ae8e-c5ef5e8cd2e8}";

fn shellex(base: &str) -> String {
    format!(r"{base}\ShellEx\{SHELLEX_KEY}")
}

fn out_of_process(clsid: &str) -> Option<Handler> {
    Some(Handler {
        clsid: clsid.into(),
        in_process: false,
    })
}

#[test]
fn finds_the_handler_on_the_extension() {
    let reg = Map::default().set(&shellex(".docx"), "", A).class(A, true);
    assert_eq!(find(&reg, "docx"), out_of_process(A));
    assert_eq!(find(&reg, ".DOCX"), out_of_process(A));
    assert_eq!(find(&reg, "xlsx"), None);
    assert_eq!(find(&reg, ""), None);
}

#[test]
fn falls_back_to_progid_associations_and_star() {
    // Through the ProgID.
    let reg = Map::default()
        .set(".msg", "", "Outlook.File.msg.15")
        .set(&shellex("Outlook.File.msg.15"), "", A)
        .class(A, true);
    assert_eq!(find(&reg, "msg"), out_of_process(A));
    // SystemFileAssociations for the extension, then its perceived type.
    let reg = Map::default()
        .set(&shellex(r"SystemFileAssociations\.vsdx"), "", A)
        .class(A, true);
    assert_eq!(find(&reg, "vsdx"), out_of_process(A));
    let reg = Map::default()
        .set(".foo", "PerceivedType", "document")
        .set(&shellex(r"SystemFileAssociations\document"), "", B)
        .class(B, true);
    assert_eq!(find(&reg, "foo"), out_of_process(B));
    // All files.
    let reg = Map::default().set(&shellex("*"), "", B).class(B, true);
    assert_eq!(find(&reg, "bar"), out_of_process(B));
}

#[test]
fn the_extension_key_comes_first() {
    let reg = Map::default()
        .set(".rtf", "", "Word.RTF.8")
        .set(&shellex(".rtf"), "", A)
        .set(&shellex("Word.RTF.8"), "", B)
        .class(A, true)
        .class(B, true);
    assert_eq!(find(&reg, "rtf"), out_of_process(A));
}

#[test]
fn ignores_junk_and_unregistered_classes() {
    let reg = Map::default().set(&shellex(".a"), "", "not a guid");
    assert_eq!(find(&reg, "a"), None);
    // The class itself isn't registered.
    let reg = Map::default().set(&shellex(".a"), "", A);
    assert_eq!(find(&reg, "a"), None);
}

#[test]
fn runs_in_process_only_when_it_has_to() {
    let in_process = Some(Handler {
        clsid: A.into(),
        in_process: true,
    });
    let base = || Map::default().set(&shellex(".a"), "", A);
    // No AppID: only a DLL to load.
    assert_eq!(find(&base().class(A, false), "a"), in_process);
    // Opts out of the surrogate.
    let reg = base()
        .class(A, true)
        .set(&format!(r"CLSID\{A}"), "DisableProcessIsolation", "1");
    assert_eq!(find(&reg, "a"), in_process);
    let reg = base()
        .class(A, true)
        .set(&format!(r"CLSID\{A}"), "DisableProcessIsolation", "0");
    assert_eq!(find(&reg, "a"), out_of_process(A));
    // A local server runs out of process anyway.
    let reg = base().set(&format!(r"CLSID\{A}\LocalServer32"), "", "h.exe");
    assert_eq!(find(&reg, "a"), out_of_process(A));
}

#[test]
fn thirty_two_bit_handlers_only_out_of_process() {
    let key = format!(r"WOW6432Node\CLSID\{A}");
    let base = || {
        Map::default().set(&shellex(".a"), "", A).set(
            &format!(r"{key}\InprocServer32"),
            "",
            "h32.dll",
        )
    };
    let reg = base().set(&key, "AppID", "{534a1e02-d58f-44f0-b58b-36cbed287c7c}");
    assert_eq!(find(&reg, "a"), out_of_process(A));
    // A 32-bit DLL can't be loaded into Oxplorer.
    assert_eq!(find(&base(), "a"), None);
}

#[test]
fn guids() {
    assert!(is_guid(A));
    assert!(!is_guid(&A[1..]));
    assert!(!is_guid("{84f66100-ff7c-4fb4-b0c0x02cd7fb668fe}"));
    assert!(!is_guid("{84f66100-ff7c-4fb4-b0c0-02cd7fb668fg}"));
}

#[test]
fn rects_in_physical_pixels() {
    assert_eq!(
        to_pixels([10.0, 20.0], [110.0, 220.0], 1.0),
        PxRect {
            x: 10,
            y: 20,
            w: 100,
            h: 200
        }
    );
    // 150 %: edges rounded, so the size follows from them.
    assert_eq!(
        to_pixels([10.5, 3.0], [101.0, 50.3], 1.5),
        PxRect {
            x: 16,
            y: 5,
            w: 136,
            h: 70
        }
    );
    // Inverted: empty, not negative.
    assert_eq!(to_pixels([50.0, 50.0], [10.0, 10.0], 2.0).w, 0);
}

#[test]
fn colorrefs() {
    assert_eq!(colorref([0x12, 0x34, 0x56]), 0x0056_3412);
}

#[test]
fn tracks_loads_and_unloads() {
    let t0 = Instant::now();
    let mut t = Tracker::default();
    assert_eq!(t.status(t0), Status::Idle);
    let a = t.open(Path::new("a.docx"), t0).unwrap();
    assert_eq!(t.status(t0), Status::Loading);
    // Opening it again doesn't load it again.
    assert_eq!(t.open(Path::new("a.docx"), t0), None);
    t.finished(a, Ok(()));
    assert_eq!(t.status(t0), Status::Shown);
    assert_eq!(t.path(), Some(Path::new("a.docx")));
    // The next file; the first one's late report changes nothing.
    let b = t.open(Path::new("b.xlsx"), t0).unwrap();
    t.finished(a, Err("late".into()));
    assert_eq!(t.status(t0), Status::Loading);
    t.finished(b, Err("broken".into()));
    assert_eq!(t.status(t0), Status::Failed("broken".into()));
    assert!(t.unload());
    assert!(!t.unload());
    assert_eq!(t.status(t0), Status::Idle);
    assert_eq!(t.path(), None);
}

#[test]
fn unloading_while_loading_ignores_the_result() {
    let t0 = Instant::now();
    let mut t = Tracker::default();
    let a = t.open(Path::new("a.docx"), t0).unwrap();
    t.unload();
    t.finished(a, Ok(()));
    assert_eq!(t.status(t0), Status::Idle);
    // The same file can be opened again.
    assert!(t.open(Path::new("a.docx"), t0).is_some());
}

#[test]
fn slow_loads_time_out() {
    let t0 = Instant::now();
    let later = t0 + TIMEOUT;
    let mut t = Tracker::default();
    t.open(Path::new("a.docx"), t0);
    assert_eq!(t.status(t0 + TIMEOUT / 2), Status::Loading);
    assert!(!t.stuck(t0 + TIMEOUT / 2));
    assert_eq!(t.status(later), Status::TimedOut);
    assert!(t.stuck(later));
    // Unloading doesn't make the thread less busy...
    t.unload();
    assert!(t.stuck(later));
    // ...a fresh one does.
    t.restarted();
    assert!(!t.stuck(later));
    let b = t.open(Path::new("b.docx"), later).unwrap();
    t.finished(b, Ok(()));
    assert!(!t.stuck(later + TIMEOUT));
    assert_eq!(t.status(later + TIMEOUT), Status::Shown);
}

/// Previews a real RTF file through the system's handler (WordPad's or
/// Word's), if there is one: `cargo test -p ox-core -- --ignored rtf`.
#[cfg(windows)]
#[test]
#[ignore = "needs a desktop session and a registered RTF preview handler"]
fn previews_rtf_through_the_system_handler() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.rtf");
    std::fs::write(&path, r"{\rtf1\ansi Hello, {\b preview}.\par}").unwrap();
    let Some(handler) = handler_for(&path) else {
        eprintln!("no preview handler for .rtf here");
        return;
    };
    let parent = win::test_window();
    let mut preview = DocumentPreview::new(parent, false, || {}).unwrap();
    preview.place(Some(PxRect {
        x: 0,
        y: 0,
        w: 400,
        h: 300,
    }));
    preview.open(&path, &handler, Colors::default());
    let start = Instant::now();
    let status = loop {
        match preview.status() {
            // The host window is this thread's: it has to handle messages.
            Status::Loading if start.elapsed() < TIMEOUT => win::pump(Duration::from_millis(50)),
            s => break s,
        }
    };
    assert_eq!(status, Status::Shown, "{handler:?}");
    preview.unload();
}
