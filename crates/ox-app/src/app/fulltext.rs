//! The text index of documents: started, reconfigured and paused from the
//! settings and the running transfers; searches take it from here.

use std::sync::Arc;

use ox_core::fulltext::{FullTextIndex, FullTextService, FullTextSettings};

use super::*;

/// The text index service and the settings it was last given.
pub struct FullText {
    pub service: FullTextService,
    /// The text index settings and skipped names last applied.
    applied: Option<(FullTextSettings, Vec<String>)>,
}

impl FullText {
    pub fn new(ctx: &Context) -> Self {
        let repaint = ctx.clone();
        Self {
            service: FullTextService::new(
                ox_core::fulltext::service::default_cache_path(),
                move || repaint.request_repaint(),
            ),
            applied: None,
        }
    }

    /// The index for a `content:` search, if it is on.
    pub fn index(&self) -> Option<Arc<FullTextIndex>> {
        self.service.index()
    }
}

impl OxApp {
    /// Give the indexer the current settings when they change, and let it
    /// wait while files are copied or moved.
    pub(super) fn manage_fulltext(&mut self) {
        let s = &self.settings.search;
        let wanted = (s.fulltext.clone(), s.index_exclude.clone());
        if self.fulltext.applied.as_ref() != Some(&wanted) {
            // Looks up the default folders: only when something changed.
            let cfg = s.fulltext.config(&s.index_exclude);
            self.fulltext.service.configure(cfg);
            self.fulltext.applied = Some(wanted);
        }
        self.fulltext.service.set_paused(self.engine.has_active());
    }
}
