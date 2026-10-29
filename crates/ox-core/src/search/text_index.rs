//! `content:` searches with the text index ([`crate::fulltext`]): the index
//! tells which files may contain the text; files it doesn't know (or that
//! changed since) are read as without it.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use crate::fulltext::{FullTextIndex, Resolved};

/// The text index a query may use, and its answer once looked up.
#[derive(Default, Clone)]
pub(super) struct TextIndex {
    index: Option<Arc<FullTextIndex>>,
    resolved: Option<Arc<Resolved>>,
}

impl TextIndex {
    pub(super) fn set(&mut self, index: Arc<FullTextIndex>) {
        self.index = Some(index);
        self.resolved = None;
    }

    /// Look the text up for the files under `roots` (anywhere if empty);
    /// on the search's thread, as it can take a moment. Whether the index
    /// is used.
    pub(super) fn resolve(&mut self, text: &str, case_sensitive: bool, roots: &[PathBuf]) -> bool {
        self.resolved = self
            .index
            .as_ref()
            .and_then(|i| i.resolve(text, case_sensitive, roots))
            .map(Arc::new);
        self.resolved.is_some()
    }

    pub(super) fn is_used(&self) -> bool {
        self.resolved.is_some()
    }

    /// The index's answer for `path`, `None` if the file has to be read.
    pub(super) fn check(
        &self,
        path: &Path,
        meta: Option<(u64, Option<SystemTime>)>,
        pattern: &regex::bytes::Regex,
    ) -> Option<bool> {
        self.resolved.as_ref()?.check(path, meta, pattern)
    }
}

impl fmt::Debug for TextIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TextIndex")
            .field("used", &self.is_used())
            .finish()
    }
}

/// Queries are compared by what they ask for, not by how they are answered.
impl PartialEq for TextIndex {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}
