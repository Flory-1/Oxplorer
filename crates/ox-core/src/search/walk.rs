//! The walk back end: a parallel directory walk that streams results.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;

use super::{Query, SearchEvent, SearchOptions};
use crate::entry::FileEntry;

pub(super) fn run(
    roots: Vec<PathBuf>,
    query: Query,
    opts: SearchOptions,
    tx: Sender<SearchEvent>,
    cancel: Arc<AtomicBool>,
    scanned: Arc<AtomicU64>,
    notify: Arc<dyn Fn() + Send + Sync>,
) {
    let started = Instant::now();
    let found = Arc::new(AtomicU64::new(0));
    let truncated = Arc::new(AtomicBool::new(false));
    let Some((first, rest)) = roots.split_first() else {
        let _ = tx.send(SearchEvent::Done {
            elapsed: started.elapsed(),
            scanned: 0,
            truncated: false,
            from_index: false,
        });
        notify();
        return;
    };

    let mut builder = ignore::WalkBuilder::new(first);
    for r in rest {
        builder.add(r);
    }
    builder
        .standard_filters(false)
        .hidden(!opts.include_hidden)
        .follow_links(false)
        .threads(opts.threads);

    let query = Arc::new(query);
    let max = opts.max_results as u64;
    builder.build_parallel().run(|| {
        let mut batch = Batch {
            items: Vec::new(),
            tx: tx.clone(),
            notify: notify.clone(),
            last_flush: Instant::now(),
            scanned: scanned.clone(),
            local_scanned: 0,
        };
        let query = query.clone();
        let cancel = cancel.clone();
        let found = found.clone();
        let truncated = truncated.clone();
        Box::new(move |result| {
            if cancel.load(Ordering::Relaxed) {
                return ignore::WalkState::Quit;
            }
            let Ok(entry) = result else {
                return ignore::WalkState::Continue;
            };
            if entry.depth() == 0 {
                return ignore::WalkState::Continue;
            }
            batch.local_scanned += 1;
            if batch.local_scanned >= 512 {
                batch.flush_scanned();
            }
            let name = entry.file_name().to_string_lossy();
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            if query.matches(&name, is_dir, || entry.metadata().ok(), entry.path()) {
                if found.fetch_add(1, Ordering::Relaxed) >= max {
                    truncated.store(true, Ordering::Relaxed);
                    cancel.store(true, Ordering::Relaxed);
                    return ignore::WalkState::Quit;
                }
                if let Ok(meta) = entry.metadata() {
                    batch
                        .items
                        .push(FileEntry::from_metadata(entry.path().to_path_buf(), &meta));
                    batch.maybe_flush();
                }
            }
            ignore::WalkState::Continue
        })
    });

    let _ = tx.send(SearchEvent::Done {
        elapsed: started.elapsed(),
        scanned: scanned.load(Ordering::Relaxed),
        truncated: truncated.load(Ordering::Relaxed),
        from_index: false,
    });
    notify();
}

/// Per-thread result buffer that is flushed periodically and on drop.
struct Batch {
    items: Vec<FileEntry>,
    tx: Sender<SearchEvent>,
    notify: Arc<dyn Fn() + Send + Sync>,
    last_flush: Instant,
    scanned: Arc<AtomicU64>,
    local_scanned: u64,
}

impl Batch {
    fn flush_scanned(&mut self) {
        self.scanned
            .fetch_add(self.local_scanned, Ordering::Relaxed);
        self.local_scanned = 0;
    }

    fn maybe_flush(&mut self) {
        if self.items.len() >= 256 || self.last_flush.elapsed() > Duration::from_millis(100) {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if !self.items.is_empty() {
            let _ = self
                .tx
                .send(SearchEvent::Results(std::mem::take(&mut self.items)));
            (self.notify)();
        }
        self.last_flush = Instant::now();
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        self.flush_scanned();
        self.flush();
    }
}
