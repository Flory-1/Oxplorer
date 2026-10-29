//! Live progress of fetch, pull and push: git writes it to stderr with
//! `--progress`, each update ending in `\r` so a terminal overwrites the
//! line ("Receiving objects:  45% (450/1000)"); other lines end in `\n`.

/// How far a long git command is: its latest progress line.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// The line as git wrote it ("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s").
    pub line: String,
    /// The percentage of the step it is in (0.0 to 1.0), if the line has one.
    pub fraction: Option<f32>,
}

/// Read a line git wrote: the percentage is the number before the first
/// `%` (git may speak the user's language, so the words aren't looked at).
pub fn parse_progress(line: &str) -> Progress {
    let line = line.trim();
    let fraction = line.find('%').and_then(|end| {
        let start = line[..end]
            .rfind(|c: char| !c.is_ascii_digit() && c != '.')
            .map_or(0, |i| i + 1);
        let percent: f32 = line[start..end].parse().ok()?;
        (0.0..=100.0).contains(&percent).then_some(percent / 100.0)
    });
    Progress {
        line: line.to_owned(),
        fraction,
    }
}

/// Splits what git writes into lines as it comes: a piece ending in `\r`
/// is a progress update the next one replaces, one ending in `\n` stays.
#[derive(Debug, Default)]
pub struct Lines {
    buf: Vec<u8>,
    /// The last piece ended with `\r` (a following `\n` makes it stay).
    after_cr: Option<String>,
}

impl Lines {
    /// Take in `bytes`; the pieces they complete as (text, stays). Empty
    /// pieces are left out.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        for &b in bytes {
            match b {
                b'\r' | b'\n' => {
                    let text = String::from_utf8_lossy(&self.buf).trim_end().to_owned();
                    self.buf.clear();
                    let cr = self.after_cr.take();
                    if text.is_empty() {
                        // "\r\n": the line before stays.
                        if let (b'\n', Some(prev)) = (b, cr) {
                            out.push((prev, true));
                        }
                        continue;
                    }
                    if b == b'\r' {
                        self.after_cr = Some(text.clone());
                    }
                    out.push((text, b == b'\n'));
                }
                _ => self.buf.push(b),
            }
        }
        out
    }

    /// What is left at the end, without a line end; it stays.
    pub fn finish(&mut self) -> Option<String> {
        let text = String::from_utf8_lossy(&std::mem::take(&mut self.buf))
            .trim_end()
            .to_owned();
        let cr = self.after_cr.take();
        match text.is_empty() {
            false => Some(text),
            // The last update ended in `\r`: it is git's final word on it.
            true => cr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_progress_lines() {
        let p = parse_progress("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s\r");
        assert_eq!(p.fraction, Some(0.45));
        assert!(p.line.starts_with("Receiving objects:  45%"));
        assert!(!p.line.ends_with('\r'));
        let p = parse_progress("remote: Counting objects: 100% (3/3), done.");
        assert_eq!(p.fraction, Some(1.0));
        // German git.
        assert_eq!(
            parse_progress("Empfange Objekte:   7% (70/1000)").fraction,
            Some(0.07)
        );
        assert_eq!(
            parse_progress("Resolving deltas: 12.5%").fraction,
            Some(0.125)
        );
        assert_eq!(parse_progress("From github.com:a/b").fraction, None);
        assert_eq!(parse_progress("odd: 250%").fraction, None);
        assert_eq!(parse_progress("100%%").fraction, Some(1.0));
    }

    #[test]
    fn splits_updates_and_lines() {
        let mut l = Lines::default();
        assert_eq!(
            l.feed(b"Receiving objects:  10% (1/10)\rReceiving objects:  50%"),
            vec![("Receiving objects:  10% (1/10)".to_owned(), false)]
        );
        // The rest of the update comes in the next read.
        assert_eq!(
            l.feed(b" (5/10)\rReceiving objects: 100% (10/10), done.\nFrom x\n"),
            vec![
                ("Receiving objects:  50% (5/10)".to_owned(), false),
                ("Receiving objects: 100% (10/10), done.".to_owned(), true),
                ("From x".to_owned(), true),
            ]
        );
        // Windows line ends: the line stays (once as an update, then for good).
        assert_eq!(
            l.feed(b"Done\r\n\r\n"),
            vec![("Done".to_owned(), false), ("Done".to_owned(), true)]
        );
        assert_eq!(l.finish(), None);
        l.feed(b"Writing objects: 100% (3/3)\r");
        assert_eq!(l.finish().as_deref(), Some("Writing objects: 100% (3/3)"));
        l.feed(b"no end");
        assert_eq!(l.finish().as_deref(), Some("no end"));
    }
}
