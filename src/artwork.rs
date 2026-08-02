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

        let bytes = client.fetch_bytes(&url).await?;
        anyhow::ensure!(!bytes.is_empty(), "empty artwork response");

        if let Err(error) = write_cached(show_id, &bytes) {
            // A read-only cache dir must not stop playback.
            debug!("could not cache artwork for show {show_id}: {error:#}");
        }
        Ok(bytes)
    }
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

/// Where a show's artwork lives on disk. Extension-less: the API serves both
/// PNG and JPEG, and the decoder sniffs the format anyway.
fn cache_path(show_id: u64) -> Option<PathBuf> {
    let mut path = glib::user_cache_dir();
    path.push("frisky-gtk");
    path.push("artwork");
    path.push(show_id.to_string());
    Some(path)
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
