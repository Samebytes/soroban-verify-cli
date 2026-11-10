//! Build-result cache keyed by commit and toolchain (wave: cli-2).
//!
//! Stores the WASM hashes produced by a build so an identical verification skips the container
//! build. The cache is local disk and trusted as such.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub type Hashes = BTreeMap<String, String>;

pub trait BuildCache {
    fn get(&self, key: &str) -> Option<Hashes>;
    fn put(&self, key: &str, hashes: &Hashes);
}

/// Key over everything that affects the build output.
pub fn cache_key(repo: &str, commit: &str, image: &str, package: Option<&str>) -> String {
    let mut h = Sha256::new();
    for part in [repo, commit, image, package.unwrap_or("")] {
        h.update(part.as_bytes());
        h.update([0]);
    }
    hex::encode(h.finalize())
}

pub struct FileCache {
    dir: PathBuf,
}

impl FileCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `SOROBAN_VERIFY_CACHE`, else the platform cache dir.
    pub fn default_dir() -> PathBuf {
        if let Ok(d) = std::env::var("SOROBAN_VERIFY_CACHE") {
            return PathBuf::from(d);
        }
        let base = std::env::var_os("LOCALAPPDATA")
            .or_else(|| std::env::var_os("XDG_CACHE_HOME"))
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(std::env::temp_dir);
        base.join("soroban-verify")
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.json"))
    }
}

impl BuildCache for FileCache {
    fn get(&self, key: &str) -> Option<Hashes> {
        let text = std::fs::read_to_string(self.path(key)).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn put(&self, key: &str, hashes: &Hashes) {
        // Best effort: a failed cache write must never fail a verification.
        if std::fs::create_dir_all(&self.dir).is_ok() {
            if let Ok(text) = serde_json::to_string(hashes) {
                let _ = std::fs::write(self.path(key), text);
            }
        }
    }
}
