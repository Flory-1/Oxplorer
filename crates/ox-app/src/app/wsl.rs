//! WSL distributions for the sidebar and the Home page, read in the
//! background now and then (they can be installed, removed, started and
//! stopped while the app runs).

use super::*;

/// How often the distributions (and which of them run) are read again.
const WSL_REFRESH: Duration = Duration::from_secs(30);

#[derive(Default)]
pub struct Wsl {
    /// The installed distributions, the default one first.
    pub distros: Vec<ox_core::wsl::Distro>,
    rx: Option<Receiver<Vec<ox_core::wsl::Distro>>>,
    at: Option<Instant>,
}

impl OxApp {
    pub(super) fn poll_wsl(&mut self) {
        let wsl = &mut self.wsl;
        if let Some(rx) = &wsl.rx {
            match rx.try_recv() {
                Ok(list) => {
                    wsl.distros = list;
                    wsl.rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
                Err(_) => wsl.rx = None,
            }
        }
        let due = wsl.at.is_none_or(|at| at.elapsed() >= WSL_REFRESH);
        if due && wsl.rx.is_none() && cfg!(windows) {
            wsl.at = Some(Instant::now());
            let (tx, rx) = crossbeam_channel::bounded(1);
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                let list = ox_core::wsl::distros();
                let _ = tx.send(list);
                ctx.request_repaint();
            });
            wsl.rx = Some(rx);
        }
    }

    /// The drives the index covers automatically: all but those mapped to a
    /// WSL distribution (reading them would keep WSL running).
    pub(super) fn index_drives(&self) -> Vec<PathBuf> {
        self.drives
            .iter()
            .filter(|d| {
                !d.remote
                    .as_deref()
                    .is_some_and(|r| ox_core::wsl::is_wsl_path(Path::new(r)))
            })
            .map(|d| d.mount.clone())
            .collect()
    }
}
