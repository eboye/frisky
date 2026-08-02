//! Cover art retrieval.
//!
//! A mix links to a show, and the show carries the 1200x1200 square art the app
//! displays. Shows repeat constantly across the schedule, so results are cached
//! on disk and re-served without touching the network.

use anyhow::{Context, Result};
use gtk::glib;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tracing::debug;

use crate::api::FriskyClient;
use crate::event::{AppEvent, Sender};

/// Tracks which shows have been requested so a channel flip does not queue the
/// same download twice.
#[derive(Clone, Default)]
pub struct ArtworkCache {
    in_flight: Arc<Mutex<HashSet<u64>>>,
}

impl ArtworkCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensures artwork for `show_id` reaches the UI, from disk if possible.
    ///
    /// Cheap and idempotent: safe to call on every metadata refresh.
    pub async fn request(&self, client: &FriskyClient, events: &Sender, show_id: u64) {
        if let Some(bytes) = read_cached(show_id) {
            debug!("artwork cache hit for show {show_id}");
            let _ = events.send(AppEvent::Artwork { show_id, bytes }).await;
            return;
        }

        {
            let mut in_flight = self.in_flight.lock().unwrap();
            if !in_flight.insert(show_id) {
                return;
            }
        }

        let result = self.download(client, show_id).await;
        self.in_flight.lock().unwrap().remove(&show_id);

        match result {
            Ok(bytes) => {
                let _ = events.send(AppEvent::Artwork { show_id, bytes }).await;
            }
            // Missing art is cosmetic — the UI keeps its placeholder rather
            // than showing an error.
            Err(error) => debug!("artwork for show {show_id} unavailable: {error:#}"),
        }
    }

    async fn download(&self, client: &FriskyClient, show_id: u64) -> Result<Vec<u8>> {
        let show = client.show(show_id).await?;
        let url = show
            .artwork_url()
            .ok_or_else(|| anyhow::anyhow!("show {show_id} has no artwork"))?
            .to_owned();

        // The URL comes from the API rather than from us, so hold it to the
        // transport we expect. Anything else would silently downgrade an
        // otherwise all-TLS app, and there is no artwork worth that.
        anyhow::ensure!(
            is_https(&url),
            "refusing non-https artwork URL for show {show_id}"
        );

        let bytes = client.fetch_bytes(&url).await?;
        anyhow::ensure!(!bytes.is_empty(), "empty artwork response");

        if let Err(error) = write_cached(show_id, &bytes) {
            // A read-only cache dir must not stop playback.
            debug!("could not cache artwork for show {show_id}: {error:#}");
        }
        prune_cache();
        Ok(bytes)
    }
}

/// Whether a URL will be fetched over TLS.
fn is_https(url: &str) -> bool {
    url.get(..8)
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://"))
}

/// `file://` URI of cached artwork, for consumers that take a URI rather than
/// bytes — MPRIS `mpris:artUrl` above all.
///
/// Returns `None` unless the file is actually on disk, so we never hand out a
/// URI that resolves to nothing.
pub fn cached_uri(show_id: u64) -> Option<String> {
    let path = cache_path(show_id)?;
    path.exists()
        .then(|| glib::filename_to_uri(&path, None).ok())
        .flatten()
        .map(|uri| uri.to_string())
}

/// Ceiling on the artwork cache. The art is 1200x1200, so this is a few dozen
/// shows — far more than the schedule cycles through, while keeping a cache
/// that is never cleaned from growing without limit over months of listening.
const CACHE_BUDGET: u64 = 64 * 1024 * 1024;

fn cache_dir() -> PathBuf {
    let mut path = glib::user_cache_dir();
    path.push("frisky-gtk");
    path.push("artwork");
    path
}

/// Where a show's artwork lives on disk. Extension-less: the API serves both
/// PNG and JPEG, and the decoder sniffs the format anyway.
fn cache_path(show_id: u64) -> Option<PathBuf> {
    Some(cache_dir().join(show_id.to_string()))
}

/// Drops the least recently modified artwork until the cache is back under
/// [`CACHE_BUDGET`].
///
/// Called after a download rather than on a timer: the cache only ever grows at
/// that moment, and a miss costs one request.
fn prune_cache() {
    prune(&cache_dir(), CACHE_BUDGET);
}

/// The pruning itself, over an explicit directory and budget so it can be
/// exercised without touching the real cache.
fn prune(dir: &std::path::Path, budget: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            if !metadata.is_file() {
                return None;
            }
            Some((metadata.modified().ok()?, metadata.len(), entry.path()))
        })
        .collect();

    let mut total: u64 = files.iter().map(|(_, size, _)| size).sum();
    if total <= budget {
        return;
    }

    // Oldest first, so the art most likely to come round again survives.
    files.sort_by_key(|(modified, _, _)| *modified);
    for (_, size, path) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(size);
            debug!("pruned cached artwork {path:?}");
        }
    }
}

fn read_cached(show_id: u64) -> Option<Vec<u8>> {
    let path = cache_path(show_id)?;
    let bytes = std::fs::read(path).ok()?;
    (!bytes.is_empty()).then_some(bytes)
}

fn write_cached(show_id: u64, bytes: &[u8]) -> Result<()> {
    let path = cache_path(show_id).context("no cache directory")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // Write then rename so a crash mid-write cannot leave a truncated image
    // that would be served as a cache hit forever.
    let temporary = path.with_extension("part");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_paths_are_per_show_and_namespaced() {
        let a = cache_path(53592).unwrap();
        let b = cache_path(53593).unwrap();
        assert_ne!(a, b);
        assert!(a.ends_with("frisky-gtk/artwork/53592"), "got {a:?}");
    }

    #[test]
    fn empty_cache_files_are_treated_as_misses() {
        // Guards the truncated-file case that the atomic write also protects
        // against.
        let path = cache_path(u64::MAX).unwrap();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(&path, b"").unwrap();
        assert_eq!(read_cached(u64::MAX), None);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn only_https_artwork_is_fetched() {
        assert!(is_https("https://example.invalid/a.png"));
        assert!(is_https("HTTPS://example.invalid/a.png"));
        // A downgrade, a local file read, and a truncated scheme.
        assert!(!is_https("http://example.invalid/a.png"));
        assert!(!is_https("file:///etc/passwd"));
        assert!(!is_https("https:/"));
        assert!(!is_https(""));
    }

    /// Over a scratch directory, never the user's real cache.
    #[test]
    fn pruning_evicts_the_oldest_until_under_budget() {
        let dir = std::env::temp_dir().join("frisky-gtk-prune-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Written oldest-first, with distinct mtimes so the order is decidable.
        for index in 0..5u64 {
            let path = dir.join(index.to_string());
            std::fs::write(&path, vec![0u8; 100]).unwrap();
            let stamp =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000 + index);
            filetime_set(&path, stamp);
        }

        // Budget fits two of the five.
        prune(&dir, 250);

        let mut left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["3", "4"], "should have kept the two newest");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Sets mtime without pulling in a dependency for it.
    fn filetime_set(path: &std::path::Path, when: std::time::SystemTime) {
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn pruning_leaves_a_cache_within_budget_alone() {
        let dir = std::env::temp_dir().join("frisky-gtk-prune-keep-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("1"), vec![0u8; 10]).unwrap();

        prune(&dir, CACHE_BUDGET);

        assert!(dir.join("1").exists(), "an under-budget cache must survive");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn round_trips_through_the_cache() {
        let show_id = u64::MAX - 1;
        write_cached(show_id, b"fake-image-bytes").unwrap();
        assert_eq!(
            read_cached(show_id).as_deref(),
            Some(&b"fake-image-bytes"[..])
        );
        let _ = std::fs::remove_file(cache_path(show_id).unwrap());
    }
}
