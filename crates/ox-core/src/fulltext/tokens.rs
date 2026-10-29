//! Words of a text, as the index stores them and as queries look them up.
//!
//! A word is a run of letters and digits (`char::is_alphanumeric`), lower-
//! cased one character at a time. That rule doesn't depend on what is
//! around a word, which is what makes the index safe to narrow `content:`
//! searches with: if a file contains the searched text, every word of the
//! text is part of a word of the file, so looking for index words that
//! *contain* each query word never loses a file (see [`query_words`]).
//!
//! There is no folding of accents (ä stays ä): the exact matcher that
//! confirms the candidates tells ä and a apart, so folding would only bring
//! more candidates, not more results.

use std::collections::HashSet;

/// Words up to this many characters are stored whole.
pub const MAX_WORD: usize = 64;
/// Longer words are stored as overlapping pieces of [`MAX_WORD`] characters
/// that start every `STEP` characters, so every part of up to `STEP`
/// characters is inside one piece; query words are cut to `STEP`.
pub const STEP: usize = MAX_WORD / 2;
/// A file with a longer run (base64, hex dumps) isn't split into words:
/// it is a candidate for every search instead.
pub const MAX_RUN: usize = 1024;

/// Lower-case `c` without context rules (a final Σ is σ), so a text and any
/// part of it are lower-cased the same way.
fn fold(c: char, out: &mut Vec<char>) {
    out.extend(c.to_lowercase());
}

/// The different words of `text`. `None` if it has a run longer than
/// [`MAX_RUN`] or more than `max_words` different words: such a file isn't
/// worth indexing word by word.
pub fn words(text: &str, max_words: usize) -> Option<HashSet<String>> {
    let mut out = HashSet::new();
    let mut run: Vec<char> = Vec::new();
    let mut chars = text.chars();
    loop {
        let c = chars.next();
        if let Some(c) = c.filter(|c| c.is_alphanumeric()) {
            fold(c, &mut run);
            if run.len() > MAX_RUN {
                return None;
            }
            continue;
        }
        if !run.is_empty() {
            add_run(&run, &mut out);
            run.clear();
            if out.len() > max_words {
                return None;
            }
        }
        if c.is_none() {
            return Some(out);
        }
    }
}

/// `run` whole, or in overlapping pieces if it is long.
fn add_run(run: &[char], out: &mut HashSet<String>) {
    if run.len() <= MAX_WORD {
        out.insert(run.iter().collect());
        return;
    }
    let mut start = 0;
    loop {
        let end = (start + MAX_WORD).min(run.len());
        out.insert(run[start..end].iter().collect());
        if end == run.len() {
            break;
        }
        start += STEP;
    }
}

/// The words of a `content:` text to look up: lower-cased, cut to
/// [`STEP`] characters (a part of a word is still part of the file's word)
/// and without repeats. A file can only contain the text if, for every one
/// of them, it has a word that contains it.
pub fn query_words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut run: Vec<char> = Vec::new();
    let mut push = |run: &mut Vec<char>| {
        if !run.is_empty() {
            let w: String = run.iter().take(STEP).collect();
            if !out.contains(&w) {
                out.push(w);
            }
            run.clear();
        }
    };
    for c in text.chars() {
        if c.is_alphanumeric() {
            fold(c, &mut run);
        } else {
            push(&mut run);
        }
    }
    push(&mut run);
    out
}

/// Does finding the one query word in a file's words already prove that
/// the file contains `text` (ignoring case)? True for a single ASCII word
/// short enough not to be cut: then no file has to be read to confirm.
pub fn is_exact_word(text: &str) -> bool {
    !text.is_empty() && text.len() <= STEP && text.bytes().all(|b| b.is_ascii_alphanumeric())
}
