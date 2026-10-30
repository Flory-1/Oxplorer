//! The inverted index: for every word, the files that contain it.
//!
//! Files get a new number each time they are (re)indexed, so the numbers in
//! a word's list only ever grow: a list is a run of delta-coded varints that
//! is only appended to. A changed or deleted file's old number is marked
//! dead and skipped; [`Store::compact`] (and saving) renumbers the living
//! files and drops the dead numbers from the lists.
//!
//! No word positions are kept: the index narrows a search down to the files
//! that have all of its words, and the exact matcher confirms them.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// What the index knows about a file's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    /// No text (a picture, a program, …): never a match.
    None,
    /// Its words are in the index.
    Words,
    /// Too many words or an endless one: a candidate for every search.
    Always,
}

#[derive(Debug, Clone)]
pub struct FileRec {
    pub path: PathBuf,
    pub size: u64,
    /// Modification time in nanoseconds since 1970 (0 = unknown).
    pub mtime: u64,
    pub text: Text,
    alive: bool,
}

/// One word's files, delta-coded.
#[derive(Debug, Default, Clone)]
struct Postings {
    last: u32,
    len: u32,
    data: Vec<u8>,
}

impl Postings {
    fn push(&mut self, id: u32) {
        let delta = if self.len == 0 { id } else { id - self.last };
        put_varint(&mut self.data, u64::from(delta));
        self.last = id;
        self.len += 1;
    }

    fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        let mut pos = 0;
        let mut cur = 0u32;
        let mut first = true;
        std::iter::from_fn(move || {
            let d = get_varint(&self.data, &mut pos)? as u32;
            cur = if first { d } else { cur + d };
            first = false;
            Some(cur)
        })
    }
}

/// Modification time of `meta` as stored.
pub fn mtime_of(modified: Option<SystemTime>) -> u64 {
    modified
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as u64)
}

/// `path` lower-cased, with `/` and `\` alike (Windows paths ignore case).
pub fn path_key(path: &Path) -> String {
    let path: PathBuf = path.components().collect();
    path.to_string_lossy().to_lowercase()
}

/// A set of file numbers.
#[derive(Debug, Clone, Default)]
pub struct Bits(Vec<u64>);

impl Bits {
    fn new(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }
    fn set(&mut self, i: u32) {
        if let Some(w) = self.0.get_mut(i as usize / 64) {
            *w |= 1 << (i % 64);
        }
    }
    pub fn get(&self, i: u32) -> bool {
        self.0
            .get(i as usize / 64)
            .is_some_and(|w| w & (1 << (i % 64)) != 0)
    }
    fn and(&mut self, other: &Bits) {
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a &= b;
        }
    }
}

#[derive(Debug, Default)]
pub struct Store {
    files: Vec<FileRec>,
    by_path: HashMap<String, u32>,
    words: HashMap<Box<str>, Postings>,
    dead: usize,
    /// The folders the last complete pass went through.
    pub roots: Vec<PathBuf>,
    /// A pass over `roots` has finished (the index can answer for them).
    pub complete: bool,
}

impl Store {
    /// Number of files known (with or without text).
    pub fn len(&self) -> usize {
        self.files.len() - self.dead
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of different words.
    pub fn word_count(&self) -> usize {
        self.words.len()
    }

    /// Files with text in the index and their total size.
    pub fn documents(&self) -> (usize, u64) {
        self.living()
            .filter(|(_, f)| f.text != Text::None)
            .fold((0, 0), |(n, b), (_, f)| (n + 1, b + f.size))
    }

    /// Rough memory use of the word lists in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.words
            .iter()
            .map(|(w, p)| w.len() + p.data.capacity() + 48)
            .sum::<usize>()
            + self.files.len() * 96
    }

    fn living(&self) -> impl Iterator<Item = (u32, &FileRec)> {
        self.files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.alive)
            .map(|(i, f)| (i as u32, f))
    }

    /// What the index has for `path`.
    pub fn get(&self, path: &Path) -> Option<&FileRec> {
        let id = *self.by_path.get(&path_key(path))?;
        Some(&self.files[id as usize])
    }

    /// Is the stored state of `path` the one on disk (same size and date)?
    pub fn is_current(&self, path: &Path, size: u64, mtime: u64) -> bool {
        self.get(path)
            .is_some_and(|f| f.size == size && f.mtime == mtime)
    }

    /// Add `path` (or replace what was stored for it). `words` are its
    /// words for [`Text::Words`], else ignored.
    pub fn put(
        &mut self,
        path: &Path,
        size: u64,
        mtime: u64,
        text: Text,
        words: impl IntoIterator<Item = String>,
    ) {
        self.remove(path);
        let id = self.files.len() as u32;
        self.files.push(FileRec {
            path: path.to_path_buf(),
            size,
            mtime,
            text,
            alive: true,
        });
        self.by_path.insert(path_key(path), id);
        if text == Text::Words {
            for w in words {
                self.words.entry(w.into_boxed_str()).or_default().push(id);
            }
        }
    }

    /// Forget `path`. Returns whether it was known.
    pub fn remove(&mut self, path: &Path) -> bool {
        let Some(id) = self.by_path.remove(&path_key(path)) else {
            return false;
        };
        self.files[id as usize].alive = false;
        self.dead += 1;
        true
    }

    /// Forget every file that `keep` says no to.
    pub fn retain(&mut self, mut keep: impl FnMut(&FileRec) -> bool) {
        let gone: Vec<PathBuf> = self
            .living()
            .filter(|(_, f)| !keep(f))
            .map(|(_, f)| f.path.clone())
            .collect();
        for p in gone {
            self.remove(&p);
        }
    }

    /// Paths of the files known, for the clean-up after a pass.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.living().map(|(_, f)| f.path.clone()).collect()
    }

    /// Renumber the files when many numbers are dead.
    pub fn compact_if_needed(&mut self) {
        if self.dead > 1024 && self.dead * 2 > self.files.len() {
            self.compact();
        }
    }

    /// Give back the room the word lists grew into while indexing.
    pub fn shrink(&mut self) {
        for p in self.words.values_mut() {
            p.data.shrink_to_fit();
        }
        self.words.shrink_to_fit();
        self.by_path.shrink_to_fit();
        self.files.shrink_to_fit();
    }

    /// Drop the dead numbers: files and word lists are rebuilt.
    pub fn compact(&mut self) {
        let mut map = vec![u32::MAX; self.files.len()];
        let mut files = Vec::with_capacity(self.len());
        for (i, f) in std::mem::take(&mut self.files).into_iter().enumerate() {
            if f.alive {
                map[i] = files.len() as u32;
                files.push(f);
            }
        }
        self.files = files;
        self.by_path = self
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (path_key(&f.path), i as u32))
            .collect();
        self.dead = 0;
        let words = std::mem::take(&mut self.words);
        for (w, p) in words {
            let mut new = Postings::default();
            for id in p.iter() {
                match map.get(id as usize) {
                    Some(&to) if to != u32::MAX => new.push(to),
                    _ => {}
                }
            }
            if new.len > 0 {
                new.data.shrink_to_fit();
                self.words.insert(w, new);
            }
        }
    }

    /// The files that may contain a text with these words (see
    /// [`super::tokens::query_words`]): for each word, the files with a word
    /// containing it, intersected; files marked [`Text::Always`] too.
    /// Also says per file whether the one word was found whole or as a
    /// part (needs no confirmation when the query is a single exact word).
    pub fn candidates(&self, query: &[String]) -> Bits {
        let n = self.files.len();
        let mut result: Option<Bits> = None;
        for q in query {
            let mut bits = Bits::new(n);
            for (w, p) in &self.words {
                if w.contains(q.as_str()) {
                    for id in p.iter() {
                        bits.set(id);
                    }
                }
            }
            match &mut result {
                Some(r) => r.and(&bits),
                None => result = Some(bits),
            }
        }
        let mut result = result.unwrap_or_else(|| Bits::new(n));
        for (i, f) in self.living() {
            if f.text == Text::Always {
                result.set(i);
            }
        }
        result
    }

    /// Every living file with its number.
    pub fn files(&self) -> impl Iterator<Item = (u32, &FileRec)> {
        self.living()
    }

    // ---- persistence -------------------------------------------------------

    const MAGIC: &'static [u8; 8] = b"OXFTI\0\0\x01";

    /// Write the index to `path` (through a temporary file, so a crash
    /// leaves the old one). Compacts first.
    pub fn save(&mut self, path: &Path) -> io::Result<()> {
        self.compact();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        {
            let mut w = BufWriter::new(fs::File::create(&tmp)?);
            let mut buf = Vec::with_capacity(1 << 16);
            buf.extend_from_slice(Self::MAGIC);
            buf.push(u8::from(self.complete));
            put_varint(&mut buf, self.roots.len() as u64);
            for r in &self.roots {
                put_str(&mut buf, &r.to_string_lossy());
            }
            put_varint(&mut buf, self.files.len() as u64);
            for f in &self.files {
                put_str(&mut buf, &f.path.to_string_lossy());
                put_varint(&mut buf, f.size);
                put_varint(&mut buf, f.mtime);
                buf.push(match f.text {
                    Text::None => 0,
                    Text::Words => 1,
                    Text::Always => 2,
                });
                if buf.len() > 1 << 16 {
                    w.write_all(&buf)?;
                    buf.clear();
                }
            }
            put_varint(&mut buf, self.words.len() as u64);
            for (word, p) in &self.words {
                put_str(&mut buf, word);
                put_varint(&mut buf, u64::from(p.len));
                put_varint(&mut buf, u64::from(p.last));
                put_varint(&mut buf, p.data.len() as u64);
                buf.extend_from_slice(&p.data);
                if buf.len() > 1 << 16 {
                    w.write_all(&buf)?;
                    buf.clear();
                }
            }
            w.write_all(&buf)?;
            w.flush()?;
        }
        fs::rename(&tmp, path)
    }

    /// Read an index written by [`Store::save`].
    pub fn load(path: &Path) -> io::Result<Store> {
        let mut data = Vec::new();
        BufReader::new(fs::File::open(path)?).read_to_end(&mut data)?;
        Self::parse(&data).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad index"))
    }

    fn parse(data: &[u8]) -> Option<Store> {
        let rest = data.strip_prefix(Self::MAGIC.as_slice())?;
        let mut pos = 1;
        let complete = *rest.first()? == 1;
        let mut s = Store {
            complete,
            ..Default::default()
        };
        for _ in 0..get_varint(rest, &mut pos)? {
            s.roots.push(PathBuf::from(get_str(rest, &mut pos)?));
        }
        let n = get_varint(rest, &mut pos)? as usize;
        s.files.reserve(n.min(1 << 24));
        for i in 0..n {
            let path = PathBuf::from(get_str(rest, &mut pos)?);
            let size = get_varint(rest, &mut pos)?;
            let mtime = get_varint(rest, &mut pos)?;
            let text = match *rest.get(pos)? {
                0 => Text::None,
                1 => Text::Words,
                2 => Text::Always,
                _ => return None,
            };
            pos += 1;
            s.by_path.insert(path_key(&path), i as u32);
            s.files.push(FileRec {
                path,
                size,
                mtime,
                text,
                alive: true,
            });
        }
        let words = get_varint(rest, &mut pos)? as usize;
        s.words.reserve(words.min(1 << 24));
        for _ in 0..words {
            let word = get_str(rest, &mut pos)?;
            let len = u32::try_from(get_varint(rest, &mut pos)?).ok()?;
            let last = u32::try_from(get_varint(rest, &mut pos)?).ok()?;
            let bytes = get_varint(rest, &mut pos)? as usize;
            let data = rest.get(pos..pos.checked_add(bytes)?)?.to_vec();
            pos += bytes;
            if last as usize >= n {
                return None;
            }
            s.words.insert(word.into(), Postings { last, len, data });
        }
        (pos == rest.len()).then_some(s)
    }
}

fn put_varint(buf: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            buf.push(b);
            return;
        }
        buf.push(b | 0x80);
    }
}

fn get_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let b = *data.get(*pos)?;
        *pos += 1;
        v |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

fn put_str(buf: &mut Vec<u8>, s: &str) {
    put_varint(buf, s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
}

fn get_str(data: &[u8], pos: &mut usize) -> Option<String> {
    let n = get_varint(data, pos)? as usize;
    let s = std::str::from_utf8(data.get(*pos..pos.checked_add(n)?)?).ok()?;
    *pos += n;
    Some(s.to_owned())
}
