//! Pictures downloaded for thumbnails in remote folders: in the background,
//! newest request first, two at a time, only pictures up to [`MAX_BYTES`],
//! and only over a connection that is already open. Leaving the folder
//! drops the waiting requests and stops the running downloads.

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use super::{RemoteAddr, RemoteFs};

/// Pictures larger than this get no thumbnail in remote folders.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

const WORKERS: usize = 2;
/// Waiting requests beyond this are dropped, oldest first.
const MAX_QUEUED: usize = 128;

/// How a request ended.
#[derive(Debug, PartialEq)]
pub enum Fetched {
    /// The file's bytes.
    Bytes(Vec<u8>),
    /// Not loaded: not connected, failed or too large. Not asked again.
    Failed,
    /// Dropped (the folder was left, or the queue overflowed): may be
    /// asked for again.
    Dropped,
}

/// Finds the open connection of a server (`None`: not connected).
pub type Lookup = dyn Fn(&RemoteAddr) -> Option<Arc<dyn RemoteFs>> + Send + Sync;
/// Gets each result with the request's tag (on a background thread).
pub type Deliver<T> = dyn Fn(T, Fetched) + Send + Sync;

struct Request<T> {
    addr: RemoteAddr,
    tag: T,
}

struct State<T> {
    queue: Vec<Request<T>>,
    /// The folder shown; requests elsewhere are dropped.
    folder: Option<RemoteAddr>,
    stopped: bool,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
    /// Changes when the folder changes: running downloads of an older
    /// generation stop.
    generation: AtomicU64,
    lookup: Box<Lookup>,
    deliver: Box<Deliver<T>>,
}

pub struct ThumbFetcher<T: Send + 'static> {
    shared: Arc<Shared<T>>,
}

/// The folder of a remote item (`/` for items at the top).
fn folder_of(addr: &RemoteAddr) -> RemoteAddr {
    addr.parent().unwrap_or_else(|| addr.with_path("/"))
}

fn in_folder(addr: &RemoteAddr, folder: &Option<RemoteAddr>) -> bool {
    folder.as_ref().is_some_and(|f| {
        let mine = folder_of(addr);
        mine.same_server(f) && mine.path.trim_end_matches('/') == f.path.trim_end_matches('/')
    })
}

impl<T: Send + 'static> ThumbFetcher<T> {
    pub fn new(
        lookup: impl Fn(&RemoteAddr) -> Option<Arc<dyn RemoteFs>> + Send + Sync + 'static,
        deliver: impl Fn(T, Fetched) + Send + Sync + 'static,
    ) -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                queue: Vec::new(),
                folder: None,
                stopped: false,
            }),
            ready: Condvar::new(),
            generation: AtomicU64::new(0),
            lookup: Box::new(lookup),
            deliver: Box::new(deliver),
        });
        for i in 0..WORKERS {
            let shared = shared.clone();
            let _ = std::thread::Builder::new()
                .name(format!("remote-thumbs-{i}"))
                .spawn(move || worker(&shared));
        }
        Self { shared }
    }

    /// Ask for the picture at `addr` (`size`: its size from the listing).
    /// Returns `false` (and nothing is delivered) for files too large or
    /// outside the folder shown.
    pub fn request(&self, addr: RemoteAddr, size: u64, tag: T) -> bool {
        let mut state = self.shared.state.lock().unwrap();
        if size > MAX_BYTES || !in_folder(&addr, &state.folder) {
            return false;
        }
        state.queue.push(Request { addr, tag });
        let dropped = if state.queue.len() > MAX_QUEUED {
            Some(state.queue.remove(0))
        } else {
            None
        };
        drop(state);
        self.shared.ready.notify_one();
        if let Some(old) = dropped {
            (self.shared.deliver)(old.tag, Fetched::Dropped);
        }
        true
    }

    /// The folder now shown (`None`: no remote folder). Requests for other
    /// folders are dropped and their downloads stopped.
    pub fn set_folder(&self, folder: Option<RemoteAddr>) {
        let mut state = self.shared.state.lock().unwrap();
        if state.folder == folder {
            return;
        }
        state.folder = folder;
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        let folder = state.folder.clone();
        let (keep, drop_now): (Vec<_>, Vec<_>) = std::mem::take(&mut state.queue)
            .into_iter()
            .partition(|r| in_folder(&r.addr, &folder));
        state.queue = keep;
        drop(state);
        for r in drop_now {
            (self.shared.deliver)(r.tag, Fetched::Dropped);
        }
    }

    /// Requests waiting.
    pub fn waiting(&self) -> usize {
        self.shared.state.lock().unwrap().queue.len()
    }
}

impl<T: Send + 'static> Drop for ThumbFetcher<T> {
    fn drop(&mut self) {
        self.shared.state.lock().unwrap().stopped = true;
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        self.shared.ready.notify_all();
    }
}

/// Collects a download; fails once it is too large or no longer wanted.
struct Sink<'a> {
    data: Vec<u8>,
    generation: u64,
    current: &'a AtomicU64,
}

impl Write for Sink<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.current.load(Ordering::SeqCst) != self.generation {
            return Err(std::io::Error::other("folder left"));
        }
        if (self.data.len() + buf.len()) as u64 > MAX_BYTES {
            return Err(std::io::Error::other("too large"));
        }
        self.data.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn worker<T: Send + 'static>(shared: &Shared<T>) {
    loop {
        let (req, generation) = {
            let mut state = shared.state.lock().unwrap();
            loop {
                if state.stopped {
                    return;
                }
                if let Some(r) = state.queue.pop() {
                    break (r, shared.generation.load(Ordering::SeqCst));
                }
                state = shared.ready.wait(state).unwrap();
            }
        };
        let Some(fs) = (shared.lookup)(&req.addr) else {
            (shared.deliver)(req.tag, Fetched::Failed);
            continue;
        };
        let mut sink = Sink {
            data: Vec::new(),
            generation,
            current: &shared.generation,
        };
        let result = fs.read_to(&req.addr.path, &mut sink);
        let fetched = if shared.generation.load(Ordering::SeqCst) != generation {
            Fetched::Dropped
        } else if result.is_ok() {
            Fetched::Bytes(sink.data)
        } else {
            Fetched::Failed
        };
        (shared.deliver)(req.tag, fetched);
    }
}

#[cfg(test)]
mod tests {
    use super::super::memory::MemoryFs;
    use super::*;
    use std::time::{Duration, Instant};

    fn addr(path: &str) -> RemoteAddr {
        RemoteAddr::parse(&format!("sftp://me@h{path}")).unwrap()
    }

    type Results = Arc<Mutex<Vec<(&'static str, Fetched)>>>;

    fn fetcher(fs: &Arc<MemoryFs>, connected: bool) -> (ThumbFetcher<&'static str>, Results) {
        let results: Results = Default::default();
        let out = results.clone();
        let fs: Arc<dyn RemoteFs> = fs.clone();
        let f = ThumbFetcher::new(
            move |_| connected.then(|| fs.clone()),
            move |tag, got| out.lock().unwrap().push((tag, got)),
        );
        (f, results)
    }

    fn wait_for(results: &Results, n: usize) {
        let start = Instant::now();
        while results.lock().unwrap().len() < n {
            assert!(start.elapsed() < Duration::from_secs(10), "timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn downloads_pictures_of_the_folder_shown() {
        let fs = Arc::new(MemoryFs::default());
        fs.add_file("/pics/a.png", b"png!");
        let (f, results) = fetcher(&fs, true);
        // Not the folder shown: refused.
        assert!(!f.request(addr("/pics/a.png"), 4, "a"));
        f.set_folder(Some(addr("/pics")));
        assert!(!f.request(addr("/pics/huge.png"), MAX_BYTES + 1, "huge"));
        assert!(f.request(addr("/pics/a.png"), 4, "a"));
        assert!(f.request(addr("/pics/missing.png"), 4, "missing"));
        wait_for(&results, 2);
        let mut r = results.lock().unwrap();
        r.sort_by_key(|(t, _)| *t);
        assert_eq!(
            *r,
            [
                ("a", Fetched::Bytes(b"png!".to_vec())),
                ("missing", Fetched::Failed)
            ]
        );
    }

    #[test]
    fn not_connected_means_no_thumbnail() {
        let fs = Arc::new(MemoryFs::default());
        fs.add_file("/a.png", b"x");
        let (f, results) = fetcher(&fs, false);
        f.set_folder(Some(addr("/")));
        assert!(f.request(addr("/a.png"), 1, "a"));
        wait_for(&results, 1);
        assert_eq!(results.lock().unwrap()[0], ("a", Fetched::Failed));
        // Nothing was downloaded.
        assert_eq!(fs.reads.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn leaving_the_folder_cancels_its_downloads() {
        let fs = Arc::new(MemoryFs::default());
        for name in ["1", "2", "3", "4", "5", "6"] {
            fs.add_file(&format!("/pics/{name}.jpg"), &[0u8; 4000]);
        }
        // 1000 blocks of 4 bytes, 2 ms each: seconds per picture.
        *fs.delay.lock().unwrap() = Some(Duration::from_millis(2));
        let (f, results) = fetcher(&fs, true);
        f.set_folder(Some(addr("/pics")));
        for (i, tag) in ["1", "2", "3", "4", "5", "6"].into_iter().enumerate() {
            assert!(f.request(addr(&format!("/pics/{}.jpg", i + 1)), 4000, tag));
        }
        // Two run, the rest wait.
        let start = Instant::now();
        while fs.reads.load(Ordering::Relaxed) < 2 {
            assert!(start.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        f.set_folder(Some(addr("/other")));
        assert_eq!(f.waiting(), 0);
        wait_for(&results, 6);
        // The running ones stopped early instead of finishing.
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(results
            .lock()
            .unwrap()
            .iter()
            .all(|(_, got)| *got == Fetched::Dropped));
        assert_eq!(fs.reads.load(Ordering::Relaxed), 2);
    }
}
