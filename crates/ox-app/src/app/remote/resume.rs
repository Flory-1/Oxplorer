//! Resuming an interrupted download, upload or copy between servers (from
//! its journal): connect to its servers in the background, then continue.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use ox_core::remote::{ConnectOptions, RemoteAddr, RemoteError};
use ox_core::transfer::journal::Pending;
use ox_core::transfer::RemoteSide;

use super::After;
use crate::app::OxApp;

enum Ready {
    Connected(Arc<Pending>, RemoteSide),
    Failed(Arc<Pending>, RemoteAddr, RemoteError),
}

pub struct ResumeState {
    /// Journals whose servers are being connected to.
    busy: HashSet<PathBuf>,
    tx: Sender<Ready>,
    rx: Receiver<Ready>,
}

impl Default for ResumeState {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            busy: HashSet::new(),
            tx,
            rx,
        }
    }
}

impl OxApp {
    /// Continue an interrupted transfer with remote paths.
    pub(crate) fn resume_remote_transfer(&mut self, pending: Arc<Pending>) {
        let state = &mut self.remote.resume;
        if !state.busy.insert(pending.path.clone()) {
            return;
        }
        let opts: Vec<(RemoteAddr, ConnectOptions)> = pending
            .servers()
            .into_iter()
            .map(|a| {
                let o = ConnectOptions::for_addr(&self.settings.remote, &a);
                (a, o)
            })
            .collect();
        let (conns, tx, ctx) = (
            self.remote.conns.clone(),
            self.remote.resume.tx.clone(),
            self.ctx.clone(),
        );
        std::thread::spawn(move || {
            let failed = std::sync::Mutex::new(None);
            let connect = |addr: &RemoteAddr| {
                let opts = opts
                    .iter()
                    .find(|(a, _)| a.same_server(addr))
                    .map(|(_, o)| o.clone())
                    .unwrap_or_default();
                conns.get(addr, &opts).inspect_err(|e| {
                    *failed.lock().unwrap() = Some((addr.clone(), e.clone()));
                })
            };
            let ready = match RemoteSide::for_request(&pending.request, &connect) {
                Ok(Some(side)) => Ready::Connected(pending, side),
                Ok(None) => return,
                Err(e) => {
                    let addr = failed.lock().unwrap().take().map(|(a, _)| a);
                    let addr = addr.unwrap_or_else(|| pending.servers().remove(0));
                    Ready::Failed(pending, addr, e)
                }
            };
            let _ = tx.send(ready);
            ctx.request_repaint();
        });
    }

    pub(super) fn poll_remote_resume(&mut self) {
        while let Ok(ready) = self.remote.resume.rx.try_recv() {
            match ready {
                Ready::Connected(pending, side) => {
                    self.remote.resume.busy.remove(&pending.path);
                    self.engine.resume_with(&pending, Some(side));
                    self.ui.show_transfers = true;
                }
                Ready::Failed(pending, addr, e) => {
                    self.remote.resume.busy.remove(&pending.path);
                    if e.needs_user() {
                        // The password or host key question; resume again after.
                        self.remote_connect(addr.clone(), After::Reload);
                        self.toast(ox_core::trf!(
                            "Connect to {server} first.",
                            server = addr.server_url()
                        ));
                    } else {
                        self.toast(format!("{}: {e}", addr.server_url()));
                    }
                }
            }
        }
    }
}
