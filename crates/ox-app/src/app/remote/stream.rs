//! Copies (and moves) from one server to another, or within a server:
//! streamed through Oxplorer by the transfer engine, one job per source
//! server.

use std::path::PathBuf;

use ox_core::remote::RemoteAddr;
use ox_core::transfer::{RemoteSide, TransferKind, TransferRequest};

use crate::app::OxApp;

impl OxApp {
    pub(super) fn remote_stream(
        &mut self,
        kind: TransferKind,
        sources: Vec<PathBuf>,
        dest: &RemoteAddr,
    ) {
        let mut by_server: Vec<(RemoteAddr, Vec<PathBuf>)> = Vec::new();
        for p in sources {
            let Some(a) = RemoteAddr::from_path(&p) else {
                continue;
            };
            match by_server.iter_mut().find(|(s, _)| s.same_server(&a)) {
                Some((_, list)) => list.push(p),
                None => by_server.push((a, vec![p])),
            }
        }
        let conflict = self.settings.transfer.default_conflict;
        for (server, items) in by_server {
            let (Some(from), Some(to)) = (
                self.remote.conns.cached(&server),
                self.remote.conns.cached(dest),
            ) else {
                let missing = if self.remote.conns.is_connected(&server) {
                    dest
                } else {
                    &server
                };
                self.toast(ox_core::trf!(
                    "Connect to {server} first.",
                    server = missing.server_url()
                ));
                continue;
            };
            let side = RemoteSide::new(from, &server).to(RemoteSide::new(to, dest));
            let req = TransferRequest {
                remote: Some(side),
                ..TransferRequest::new(kind, items, dest.to_path(), conflict)
            };
            self.engine.submit(req);
            self.ui.show_transfers = true;
        }
    }
}
