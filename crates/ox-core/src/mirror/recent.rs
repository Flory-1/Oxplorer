//! Folders mirrored recently, with their options, offered again in the
//! mirror dialog. Stored in `mirror_pairs.toml` in the config folder; every
//! change reads the file again first (several windows), like the search
//! history.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::Options;

/// Pairs remembered at most.
pub const MAX_PAIRS: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentPair {
    pub source: PathBuf,
    pub target: PathBuf,
    #[serde(default)]
    pub options: Options,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecentPairs {
    /// Newest first.
    #[serde(default)]
    pub pairs: Vec<RecentPair>,
}

fn same(a: &Path, b: &Path) -> bool {
    a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

impl RecentPairs {
    pub fn path() -> PathBuf {
        crate::config_dir().join("mirror_pairs.toml")
    }

    pub fn load(path: &Path) -> Self {
        let mut r: Self = crate::settings::load_toml(path).unwrap_or_default();
        r.pairs.truncate(MAX_PAIRS);
        r
    }

    /// A mirror was started: remember it first (read again and written back).
    pub fn remember(path: &Path, pair: RecentPair) -> io::Result<Self> {
        let mut r = Self::load(path);
        r.add(pair);
        crate::settings::save_toml(path, &r)?;
        Ok(r)
    }

    pub fn add(&mut self, pair: RecentPair) {
        self.pairs
            .retain(|p| !(same(&p.source, &pair.source) && same(&p.target, &pair.target)));
        self.pairs.insert(0, pair);
        self.pairs.truncate(MAX_PAIRS);
    }

    /// The newest pair with `source` as its source.
    pub fn for_source(&self, source: &Path) -> Option<&RecentPair> {
        self.pairs.iter().find(|p| same(&p.source, source))
    }
}
