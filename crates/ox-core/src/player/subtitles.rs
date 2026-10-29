//! Subtitles: `.srt` and `.vtt` files next to a video, their cues and the
//! cue to show at a playback position.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// A piece of a cue's line in one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub italic: bool,
    pub bold: bool,
}

/// One subtitle: shown from `start` until `end`, one or more lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start: Duration,
    pub end: Duration,
    pub lines: Vec<Vec<Span>>,
}

impl Cue {
    /// The cue's text without styles, lines joined by `\n`.
    pub fn plain(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.iter().map(|s| s.text.as_str()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The cues of a subtitle file, by start time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Subtitles {
    cues: Vec<Cue>,
}

impl Subtitles {
    /// Parse SubRip (`.srt`) or WebVTT (`.vtt`) text; both are blocks of a
    /// timing line (`00:00:01,000 --> 00:00:02,500`) and text lines.
    pub fn parse(text: &str) -> Self {
        let mut cues = Vec::new();
        let mut current: Option<(Duration, Duration, Vec<&str>)> = None;
        let finish = |cur: Option<(Duration, Duration, Vec<&str>)>, cues: &mut Vec<Cue>| {
            if let Some((start, end, lines)) = cur {
                let lines: Vec<Vec<Span>> = lines
                    .iter()
                    .map(|l| styled(l))
                    .filter(|l| !l.is_empty())
                    .collect();
                if !lines.is_empty() {
                    cues.push(Cue { start, end, lines });
                }
            }
        };
        for line in text.lines() {
            let line = line.trim_end_matches('\r');
            if let Some((start, end)) = timing(line) {
                if let Some((_, _, lines)) = &mut current {
                    // A cue number right before the timing line, without a
                    // blank line between the cues.
                    if lines.last().is_some_and(|l| is_number(l)) {
                        lines.pop();
                    }
                }
                finish(current.take(), &mut cues);
                current = Some((start, end, Vec::new()));
            } else if line.trim().is_empty() {
                finish(current.take(), &mut cues);
            } else if let Some((_, _, lines)) = &mut current {
                lines.push(line);
            }
        }
        finish(current, &mut cues);
        cues.sort_by_key(|c| c.start);
        Self { cues }
    }

    /// Read a subtitle file (see [`decode`] for its encoding).
    pub fn read(path: &Path) -> io::Result<Self> {
        Ok(Self::parse(&decode(&std::fs::read(path)?)))
    }

    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// The cues shown at `t` (usually one; overlapping cues are all shown).
    pub fn at(&self, t: Duration) -> impl Iterator<Item = &Cue> {
        let started = self.cues.partition_point(|c| c.start <= t);
        self.cues[..started].iter().filter(move |c| t < c.end)
    }
}

fn is_number(line: &str) -> bool {
    let l = line.trim();
    !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit())
}

/// `00:01:02,345 --> 00:01:04.000 align:start` → start and end.
fn timing(line: &str) -> Option<(Duration, Duration)> {
    let (a, b) = line.split_once("-->")?;
    let end = b.split_whitespace().next()?;
    Some((timestamp(a.trim())?, timestamp(end)?))
}

/// `01:02:03,456`, `01:02:03.456` or (WebVTT) `02:03.456`.
fn timestamp(s: &str) -> Option<Duration> {
    let (clock, frac) = s.split_once([',', '.']).unwrap_or((s, "0"));
    let parts: Vec<u64> = clock
        .split(':')
        .map(|p| p.trim().parse().ok())
        .collect::<Option<_>>()?;
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (*h, *m, *s),
        [m, s] => (0, *m, *s),
        _ => return None,
    };
    if !frac.bytes().all(|b| b.is_ascii_digit()) || frac.is_empty() {
        return None;
    }
    // "5" is 500 ms, "05" 50 ms, "005" 5 ms.
    let digits = &frac[..frac.len().min(3)];
    let ms = digits.parse::<u64>().ok()? * 10u64.pow(3 - digits.len() as u32);
    Some(Duration::from_millis(((h * 60 + m) * 60 + sec) * 1000 + ms))
}

/// A line with `<i>` and `<b>` turned into styles; other tags (`<font>`,
/// `<u>`, WebVTT's `<c>`, `<v>` and timestamps), `{\an8}`-style overrides
/// and the HTML entities WebVTT uses are taken out.
fn styled(line: &str) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let (mut italic, mut bold) = (false, false);
    let mut text = String::new();
    let mut flush = |text: &mut String, italic: bool, bold: bool| {
        if text.is_empty() {
            return;
        }
        let t = entities(text);
        match spans.last_mut() {
            Some(last) if last.italic == italic && last.bold == bold => last.text.push_str(&t),
            _ => spans.push(Span {
                text: t,
                italic,
                bold,
            }),
        }
        text.clear();
    };
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let close = match c {
            '<' => rest.find('>'),
            '{' if rest.starts_with("{\\") => rest.find('}'),
            _ => None,
        };
        if let Some(end) = close {
            let tag = rest[1..end].trim().to_ascii_lowercase();
            let on = !tag.starts_with('/');
            match tag.trim_start_matches('/') {
                "i" => {
                    flush(&mut text, italic, bold);
                    italic = on;
                }
                "b" => {
                    flush(&mut text, italic, bold);
                    bold = on;
                }
                _ => {}
            }
            rest = &rest[end + 1..];
        } else {
            text.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    flush(&mut text, italic, bold);
    if spans.iter().all(|s| s.text.trim().is_empty()) {
        return Vec::new();
    }
    spans
}

fn entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", "\u{a0}")
        .replace("&lrm;", "")
        .replace("&rlm;", "")
        .replace("&amp;", "&")
}

/// Text of a subtitle or playlist file: UTF-8 (with or without BOM),
/// UTF-16 with BOM, or else Windows-1252.
pub fn decode(bytes: &[u8]) -> String {
    use crate::textinfo::{self, Encoding};
    match textinfo::encoding(bytes) {
        Encoding::Other => bytes.iter().map(|&b| cp1252(b)).collect(),
        enc => textinfo::decode(bytes, enc),
    }
}

/// A Windows-1252 byte as a character (the five unused bytes as C1
/// controls, like Windows does).
fn cp1252(b: u8) -> char {
    const HIGH: [u16; 32] = [
        0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160,
        0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013,
        0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
    ];
    match b {
        0x80..=0x9F => char::from_u32(HIGH[(b - 0x80) as usize] as u32).unwrap_or('?'),
        _ => b as char,
    }
}

/// A subtitle file for a video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub path: PathBuf,
    /// What is between the video's name and the extension ("en",
    /// "de.forced"); empty for `video.srt`.
    pub label: String,
}

/// Subtitle files among `names` (the file names in the video's folder)
/// that belong to `video`: `video.srt`, `video.vtt`, `video.en.srt`, …
/// The one without a label comes first, then by label.
pub fn find(video: &Path, names: &[String]) -> Vec<Track> {
    let Some(stem) = video.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
        return Vec::new();
    };
    let folder = video.parent().unwrap_or(Path::new(""));
    let mut tracks: Vec<Track> = names
        .iter()
        .filter_map(|name| {
            let (n, s) = (name.len(), stem.len());
            if n < s + 4 || !name.is_char_boundary(s) || !name.is_char_boundary(n - 4) {
                return None;
            }
            let ext = name[n - 4..].to_ascii_lowercase();
            if !(ext == ".srt" || ext == ".vtt") || !name[..s].eq_ignore_ascii_case(&stem) {
                return None;
            }
            let label = match &name[s..n - 4] {
                "" => "",
                rest => rest.strip_prefix('.')?,
            };
            Some(Track {
                path: folder.join(name),
                label: label.to_owned(),
            })
        })
        .collect();
    tracks.sort_by_key(|t| (!t.label.is_empty(), t.label.to_lowercase(), t.path.clone()));
    tracks
}

/// [`find`] in the video's folder.
pub fn find_next_to(video: &Path) -> Vec<Track> {
    let Some(folder) = video.parent() else {
        return Vec::new();
    };
    let names: Vec<String> = std::fs::read_dir(folder)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    find(video, &names)
}

/// The track to show: one in the interface language (`german` or else
/// English), else the first.
pub fn preferred(tracks: &[Track], german: bool) -> Option<usize> {
    let codes: &[&str] = match german {
        true => &["de", "deu", "ger", "german", "deutsch"],
        false => &["en", "eng", "english"],
    };
    let matches = |t: &Track| {
        t.label.split(['.', '_', ' ']).any(|part| {
            let part = part.to_lowercase();
            let lang = part.split('-').next().unwrap_or("");
            codes.contains(&lang)
        })
    };
    tracks
        .iter()
        .position(matches)
        .or((!tracks.is_empty()).then_some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn srt_cues_with_several_lines_and_tags() {
        let text = "\u{feff}1\r\n00:00:01,000 --> 00:00:02,500\r\nHello <i>there</i>\r\nSecond line\r\n\r\n\
                    2\n00:00:03,000 --> 00:00:04,000\n<b>Bold</b> &amp; {\\an8}<font color=\"red\">red</font>\n\
                    3\n00:00:05,000 --> 00:00:06,000\n42\n";
        let subs = Subtitles::parse(text);
        let cues = subs.cues();
        assert_eq!(cues.len(), 3);
        assert_eq!((cues[0].start, cues[0].end), (ms(1000), ms(2500)));
        assert_eq!(cues[0].plain(), "Hello there\nSecond line");
        assert_eq!(
            cues[0].lines[0],
            vec![
                Span {
                    text: "Hello ".into(),
                    italic: false,
                    bold: false
                },
                Span {
                    text: "there".into(),
                    italic: true,
                    bold: false
                },
            ]
        );
        // No blank line before the next cue: its number isn't text.
        assert_eq!(cues[1].plain(), "Bold & red");
        assert!(cues[1].lines[0][0].bold);
        assert_eq!(cues[2].plain(), "42");
    }

    #[test]
    fn vtt_with_header_settings_and_short_times() {
        let text = "WEBVTT - Title\n\nNOTE a comment\nmore\n\nintro\n00:01.500 --> 00:03.000 align:start position:10%\n<v Anna>Hi &lt;3</v>\n\n01:00:00.000 --> 01:00:01.000\n<c.yellow>Late</c>\n";
        let subs = Subtitles::parse(text);
        assert_eq!(subs.cues().len(), 2);
        assert_eq!(subs.cues()[0].start, ms(1500));
        assert_eq!(subs.cues()[0].plain(), "Hi <3");
        assert_eq!(subs.cues()[1].start, ms(3_600_000));
        assert_eq!(subs.cues()[1].plain(), "Late");
    }

    #[test]
    fn cue_lookup_by_time() {
        let subs = Subtitles::parse(
            "1\n00:00:01,000 --> 00:00:03,000\nA\n\n2\n00:00:02,000 --> 00:00:04,000\nB\n\n3\n00:00:10,000 --> 00:00:11,000\nC\n",
        );
        let at = |t| subs.at(ms(t)).map(Cue::plain).collect::<Vec<_>>();
        assert!(at(500).is_empty());
        assert_eq!(at(1000), ["A"]);
        assert_eq!(at(2500), ["A", "B"]);
        assert_eq!(at(3000), ["B"]);
        assert!(at(5000).is_empty());
        assert_eq!(at(10_999), ["C"]);
        assert!(at(11_000).is_empty());
    }

    #[test]
    fn timestamps() {
        assert_eq!(timestamp("00:00:01,5"), Some(ms(1500)));
        assert_eq!(timestamp("00:00:01,05"), Some(ms(1050)));
        assert_eq!(timestamp("1:02:03.004"), Some(ms(3_723_004)));
        assert_eq!(timestamp("02:03"), Some(ms(123_000)));
        assert_eq!(timestamp("x:00:01,000"), None);
        assert_eq!(timing("00:00:01,000 -> 00:00:02,000"), None);
    }

    #[test]
    fn encodings() {
        assert_eq!(decode("\u{feff}Grüße".as_bytes()), "Grüße");
        assert_eq!(decode("Grüße".as_bytes()), "Grüße");
        // Windows-1252: ü, ß, the euro sign and typographic quotes.
        assert_eq!(
            decode(&[b'G', 0xFC, 0xDF, b' ', 0x80, b' ', 0x84, b'x', 0x93]),
            "Gü\u{df} \u{20ac} \u{201e}x\u{201c}"
        );
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain("Hi".encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(decode(&utf16), "Hi");
    }

    #[test]
    fn tracks_next_to_the_video() {
        let names: Vec<String> = [
            "Movie.mkv",
            "Movie.de.srt",
            "Movie.srt",
            "movie.EN.vtt",
            "Movie 2.srt",
            "Movie.de.forced.srt",
            "Other.srt",
            "Movie.txt",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let video = Path::new("videos").join("Movie.mkv");
        let tracks = find(&video, &names);
        let labels: Vec<&str> = tracks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["", "de", "de.forced", "EN"]);
        assert_eq!(tracks[0].path, Path::new("videos").join("Movie.srt"));
        assert_eq!(preferred(&tracks, true), Some(1));
        assert_eq!(preferred(&tracks, false), Some(3));
        assert_eq!(preferred(&tracks[..1], true), Some(0));
        assert_eq!(preferred(&[], true), None);
        let pt = [Track {
            path: "a.pt-BR.srt".into(),
            label: "pt-BR".into(),
        }];
        assert_eq!(preferred(&pt, false), Some(0));
    }
}
