//! The words of the index, kept compact: hundreds of thousands of them are
//! normal, so per-word heap allocations would cost several times the words
//! themselves.
//!
//! All words are in one string; each has a fixed-size record with its place
//! in that string and its file list. Most lists are a few bytes (the word is
//! in one or two files) and stay inside the record; only longer ones go to
//! the heap. The lookup table holds just record numbers.

use std::hash::BuildHasher;

use hashbrown::{DefaultHashBuilder, HashTable};

use super::store::{encode_varint, get_varint};

/// File lists up to this many bytes are kept in the word's record.
const INLINE: usize = 14;

/// The bytes of a file list: in place while short, else on the heap.
#[derive(Debug, Clone)]
enum Bytes {
    Inline {
        len: u8,
        buf: [u8; INLINE],
    },
    // Boxed so this stays 16 bytes (a `Vec` is 24) in every word's record;
    // only the few long lists pay for the extra allocation.
    #[allow(clippy::box_collection)]
    Heap(Box<Vec<u8>>),
}

impl Default for Bytes {
    fn default() -> Self {
        Self::Inline {
            len: 0,
            buf: [0; INLINE],
        }
    }
}

impl Bytes {
    fn from_vec(v: Vec<u8>) -> Self {
        if v.len() > INLINE {
            return Self::Heap(Box::new(v));
        }
        let mut buf = [0; INLINE];
        buf[..v.len()].copy_from_slice(&v);
        Self::Inline {
            len: v.len() as u8,
            buf,
        }
    }

    fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { len, buf } => &buf[..*len as usize],
            Self::Heap(v) => v,
        }
    }

    fn extend(&mut self, more: &[u8]) {
        match self {
            Self::Inline { len, buf } => {
                let n = *len as usize;
                if n + more.len() <= INLINE {
                    buf[n..n + more.len()].copy_from_slice(more);
                    *len += more.len() as u8;
                } else {
                    let mut v = Vec::with_capacity(2 * (n + more.len()));
                    v.extend_from_slice(&buf[..n]);
                    v.extend_from_slice(more);
                    *self = Self::Heap(Box::new(v));
                }
            }
            Self::Heap(v) => v.extend_from_slice(more),
        }
    }

    fn shrink(&mut self) {
        if let Self::Heap(v) = self {
            v.shrink_to_fit();
        }
    }

    /// Bytes on the heap (the list and its `Vec`).
    fn heap_bytes(&self) -> usize {
        match self {
            Self::Inline { .. } => 0,
            Self::Heap(v) => std::mem::size_of::<Vec<u8>>() + v.capacity(),
        }
    }
}

/// One word's files, delta-coded.
#[derive(Debug, Default, Clone)]
pub(super) struct Postings {
    pub last: u32,
    pub len: u32,
    data: Bytes,
}

impl Postings {
    pub fn new(last: u32, len: u32, data: Vec<u8>) -> Self {
        Self {
            last,
            len,
            data: Bytes::from_vec(data),
        }
    }

    pub fn push(&mut self, id: u32) {
        let delta = if self.len == 0 { id } else { id - self.last };
        let mut buf = [0; 10];
        let n = encode_varint(u64::from(delta), &mut buf);
        self.data.extend(&buf[..n]);
        self.last = id;
        self.len += 1;
    }

    pub fn bytes(&self) -> &[u8] {
        self.data.as_slice()
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        let data = self.data.as_slice();
        let mut pos = 0;
        let mut cur = 0u32;
        let mut first = true;
        std::iter::from_fn(move || {
            let d = get_varint(data, &mut pos)? as u32;
            cur = if first { d } else { cur + d };
            first = false;
            Some(cur)
        })
    }
}

#[derive(Debug, Clone)]
struct Word {
    /// Byte range in [`Words::text`].
    off: u32,
    len: u16,
    postings: Postings,
}

/// Every word with its file list.
#[derive(Debug, Default, Clone)]
pub(super) struct Words {
    text: String,
    list: Vec<Word>,
    /// Numbers into `list`, found by the word.
    table: HashTable<u32>,
    hasher: DefaultHashBuilder,
}

fn word_at<'a>(text: &'a str, w: &Word) -> &'a str {
    &text[w.off as usize..w.off as usize + w.len as usize]
}

impl Words {
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn reserve(&mut self, words: usize) {
        let Self {
            text,
            list,
            table,
            hasher,
        } = self;
        list.reserve(words);
        table.reserve(words, |&i| {
            hasher.hash_one(word_at(text, &list[i as usize]))
        });
    }

    /// The file list of `word`, new and empty if it isn't known yet.
    pub fn entry(&mut self, word: &str) -> &mut Postings {
        let hash = self.hasher.hash_one(word);
        let Self {
            text,
            list,
            table,
            hasher,
        } = self;
        let found = table
            .find(hash, |&i| word_at(text, &list[i as usize]) == word)
            .copied();
        let i = match found {
            Some(i) => i,
            None => {
                let i = list.len() as u32;
                list.push(Word {
                    off: text.len() as u32,
                    len: word.len() as u16,
                    postings: Postings::default(),
                });
                text.push_str(word);
                table.insert_unique(hash, i, |&j| {
                    hasher.hash_one(word_at(text, &list[j as usize]))
                });
                i
            }
        };
        &mut self.list[i as usize].postings
    }

    /// Add a word that isn't in yet (loading, compacting).
    pub fn insert_new(&mut self, word: &str, postings: Postings) {
        *self.entry(word) = postings;
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Postings)> {
        self.list
            .iter()
            .map(|w| (word_at(&self.text, w), &w.postings))
    }

    pub fn shrink_to_fit(&mut self) {
        for w in &mut self.list {
            w.postings.data.shrink();
        }
        let Self {
            text,
            list,
            table,
            hasher,
        } = self;
        text.shrink_to_fit();
        list.shrink_to_fit();
        table.shrink_to_fit(|&i| hasher.hash_one(word_at(text, &list[i as usize])));
    }

    /// Rough memory use in bytes.
    pub fn memory_bytes(&self) -> usize {
        self.text.capacity()
            + self.list.capacity() * std::mem::size_of::<Word>()
            + self.table.capacity() * (std::mem::size_of::<u32>() + 1)
            + self
                .list
                .iter()
                .map(|w| w.postings.data.heap_bytes())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_stay_small() {
        assert!(std::mem::size_of::<Word>() <= 32);
    }

    #[test]
    fn lists_move_to_the_heap_and_keep_their_numbers() {
        let mut words = Words::default();
        let ids: Vec<u32> = (0..200).map(|i| i * 300).collect();
        for &id in &ids {
            words.entry("often").push(id);
        }
        words.entry("once").push(7);
        words.shrink_to_fit();
        let got: Vec<(String, Vec<u32>)> = words
            .iter()
            .map(|(w, p)| (w.to_owned(), p.iter().collect()))
            .collect();
        assert_eq!(
            got,
            vec![("often".to_owned(), ids), ("once".to_owned(), vec![7])]
        );
        assert_eq!(words.len(), 2);
        assert!(matches!(words.list[1].postings.data, Bytes::Inline { .. }));
    }
}
