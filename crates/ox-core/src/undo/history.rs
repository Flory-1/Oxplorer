//! The undo and redo stacks.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{Entry, LIMIT};

/// An entry on one of the stacks. `id` tells it apart from the entries of
/// other windows when the history is saved; `at` is when it got there
/// (Unix milliseconds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub id: u64,
    pub at: i64,
    pub entry: Entry,
}

impl Record {
    pub fn new(entry: Entry) -> Self {
        Self {
            id: new_id(),
            at: now_ms(),
            entry,
        }
    }
}

pub(super) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Unique enough between the windows: the time, the process and a counter.
fn new_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (u64::from(std::process::id()) << 32)
        ^ n.wrapping_mul(0xD6E8_FEB8_6659_FD93)
}

/// The undo and redo stacks.
#[derive(Debug, Default)]
pub struct History {
    pub(super) undo: VecDeque<Record>,
    pub(super) redo: Vec<Record>,
    /// Something was recorded since the last [`History::take`].
    recorded: bool,
    /// Changed since the last [`History::take_changed`] (to be saved).
    changed: bool,
}

impl History {
    /// A history with these records, oldest first; at most [`LIMIT`] each.
    pub fn from_records(undo: Vec<Record>, redo: Vec<Record>) -> Self {
        let mut h = Self {
            undo: undo.into(),
            redo,
            ..Self::default()
        };
        h.trim();
        h
    }

    fn trim(&mut self) {
        while self.undo.len() > LIMIT {
            self.undo.pop_front();
        }
        if self.redo.len() > LIMIT {
            self.redo.drain(..self.redo.len() - LIMIT);
        }
    }

    /// A new operation was done: it can be undone, and nothing can be redone.
    pub fn record(&mut self, entry: Entry) {
        self.redo.clear();
        self.recorded = true;
        self.push_undo(entry);
    }

    fn push_undo(&mut self, entry: Entry) {
        self.changed = true;
        self.undo.push_back(Record::new(entry));
        self.trim();
    }

    fn push_redo(&mut self, entry: Entry) {
        self.changed = true;
        self.redo.push(Record::new(entry));
        self.trim();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The entry that would be undone (`redo` false) or redone next.
    pub fn peek(&self, redo: bool) -> Option<&Entry> {
        if redo {
            self.redo.last().map(|r| &r.entry)
        } else {
            self.undo.back().map(|r| &r.entry)
        }
    }

    /// The entry to undo (`redo` false) or redo next, taken off its stack.
    pub fn take(&mut self, redo: bool) -> Option<Entry> {
        self.recorded = false;
        let record = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        self.changed |= record.is_some();
        record.map(|r| r.entry)
    }

    /// Put an entry back where [`History::take`] got it (it couldn't be reversed).
    pub fn put_back(&mut self, redo: bool, entry: Entry) {
        if redo {
            self.push_redo(entry);
        } else {
            self.push_undo(entry);
        }
    }

    /// An entry was undone or redone; `entry.op` is what that did. It goes
    /// onto the other stack (unless something new was done meanwhile, which
    /// ends what can be redone).
    pub fn reversed(&mut self, redo: bool, entry: Entry) {
        if redo {
            self.push_undo(entry);
        } else if !self.recorded {
            self.push_redo(entry);
        }
    }

    /// The records, oldest first: (undo, redo).
    pub fn records(&self) -> (impl Iterator<Item = &Record>, impl Iterator<Item = &Record>) {
        (self.undo.iter(), self.redo.iter())
    }

    /// Keep only the records `keep` says yes to.
    pub fn retain(&mut self, mut keep: impl FnMut(&Record) -> bool) {
        self.undo.retain(&mut keep);
        self.redo.retain(keep);
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.changed |= self.can_undo() || self.can_redo();
        self.undo.clear();
        self.redo.clear();
    }

    /// Whether the history changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}
