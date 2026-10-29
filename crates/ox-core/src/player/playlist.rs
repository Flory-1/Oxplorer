//! Playlists: `.m3u` and `.m3u8` files (paths, one per line, with
//! `#EXTINF` lines giving the titles).

use std::io;
use std::path::{Path, PathBuf};

use super::queue::Item;

/// Whether the file is a playlist the player opens (by its extension).
pub fn is_playlist(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("m3u") || e.eq_ignore_ascii_case("m3u8"))
}

/// Read a playlist; relative paths are taken from its folder.
pub fn read(path: &Path) -> io::Result<Vec<Item>> {
    let text = super::subtitles::decode(&std::fs::read(path)?);
    Ok(parse_m3u(&text, path.parent().unwrap_or(Path::new(""))))
}

/// The files of an M3U playlist. `base` is the folder relative paths are
/// relative to. Web addresses (`http://…`) are left out.
pub fn parse_m3u(text: &str, base: &Path) -> Vec<Item> {
    let mut items = Vec::new();
    let mut title: Option<String> = None;
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            title = extinf_title(info);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        if let Some(path) = entry_path(line, base) {
            items.push(Item {
                path,
                title: title.take(),
            });
        }
        title = None;
    }
    items
}

/// `123 tvg-name="a, b",Artist - Title` → "Artist - Title": the text after
/// the first comma that isn't inside quotes.
fn extinf_title(info: &str) -> Option<String> {
    let mut quoted = false;
    for (i, c) in info.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                let t = info[i + 1..].trim();
                return (!t.is_empty()).then(|| t.to_owned());
            }
            _ => {}
        }
    }
    None
}

fn entry_path(line: &str, base: &Path) -> Option<PathBuf> {
    let line = match line.get(..8) {
        Some(p) if p.eq_ignore_ascii_case("file:///") => {
            let rest = percent_decode(&line[8..]);
            // file:///C:/… on Windows, file:///home/… elsewhere.
            return Some(PathBuf::from(if rest.as_bytes().get(1) == Some(&b':') {
                rest
            } else {
                format!("/{rest}")
            }));
        }
        _ => line,
    };
    // Other schemes (http://, rtsp://, …); not a drive letter like C:\.
    if let Some((scheme, _)) = line.split_once("://") {
        if scheme.len() > 1 {
            return None;
        }
    }
    let native = line.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR);
    Some(base.join(native))
}

/// `%20` → space (UTF-8 bytes put together).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (
            b[i],
            b.get(i + 1).and_then(|&c| hex(c)),
            b.get(i + 2).and_then(|&c| hex(c)),
        ) {
            (b'%', Some(h), Some(l)) => {
                out.push((h * 16 + l) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extended_m3u_with_titles_and_relative_paths() {
        let base = Path::new("music");
        let text = "\u{feff}#EXTM3U\r\n#EXTINF:215,Band - Song, Part 1\r\nalbum/01 Song.mp3\r\n\r\n\
                    # a comment\n02.flac\n#EXTINF:-1 tvg-name=\"x, y\",Radio\nhttp://example.com/stream\n\
                    #EXTINF:10,\n..\\other\\03.ogg\n";
        let items = parse_m3u(text, base);
        let paths: Vec<PathBuf> = items.iter().map(|i| i.path.clone()).collect();
        assert_eq!(
            paths,
            [
                base.join("album").join("01 Song.mp3"),
                base.join("02.flac"),
                base.join("..").join("other").join("03.ogg"),
            ]
        );
        assert_eq!(items[0].title.as_deref(), Some("Band - Song, Part 1"));
        // The title of a left-out web address doesn't go to the next file.
        assert_eq!(items[1].title, None);
        assert_eq!(items[2].title, None);
        assert_eq!(items[2].label(), "03.ogg");
        assert_eq!(
            extinf_title("-1 tvg-name=\"x, y\",Radio"),
            Some("Radio".into())
        );
    }

    #[test]
    fn absolute_paths_and_file_urls() {
        let items = parse_m3u(
            "file:///C:/My%20Music/a%C3%A4.mp3\nfile:///home/me/b.mp3\n",
            Path::new("x"),
        );
        assert_eq!(items[0].path, PathBuf::from("C:/My Music/aä.mp3"));
        assert_eq!(items[1].path, PathBuf::from("/home/me/b.mp3"));
        #[cfg(windows)]
        {
            let items = parse_m3u("D:\\Music\\c.mp3\n", Path::new("C:\\lists"));
            assert_eq!(items[0].path, PathBuf::from("D:\\Music\\c.mp3"));
        }
        #[cfg(not(windows))]
        {
            let items = parse_m3u("/srv/music/c.mp3\n", Path::new("/lists"));
            assert_eq!(items[0].path, PathBuf::from("/srv/music/c.mp3"));
        }
        assert!(is_playlist(Path::new("a.M3U8")));
        assert!(!is_playlist(Path::new("a.mp3")));
    }
}
