//! The one-way channel from background work into the GTK main loop.
//!
//! Nothing in here may hold a GTK type: GDK/GTK objects are not `Send`, so
//! artwork travels as raw bytes and is decoded into a `gdk::Texture` on the
//! main thread.

use crate::api::model::{Mix, ScheduleEntry};
use crate::channel::Channel;

pub type Sender = async_channel::Sender<AppEvent>;
pub type Receiver = async_channel::Receiver<AppEvent>;

pub fn channel() -> (Sender, Receiver) {
    async_channel::unbounded()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerState {
    Stopped,
    Buffering,
    Playing,
}

/// What is on air for one channel, with the link fields already resolved.
#[derive(Debug, Clone)]
pub struct NowPlaying {
    pub channel: Channel,
    pub mix: Mix,
    /// Resolved from `artist_id`; `None` while the lookup is in flight or if it
    /// failed.
    pub artist: Option<String>,
    pub show_id: Option<u64>,
}

impl NowPlaying {
    /// Display title, preferring "Show - Artist" over the raw mix title, which
    /// carries a date that is noise in a now-playing line.
    pub fn display_title(&self) -> String {
        self.mix.title.clone()
    }

    pub fn subtitle(&self) -> String {
        let genre = self.mix.genre.join(", ");
        match (&self.artist, genre.is_empty()) {
            (Some(artist), false) => format!("{artist} · {genre}"),
            (Some(artist), true) => artist.clone(),
            (None, false) => genre,
            (None, true) => self.channel.title().to_owned(),
        }
    }
}

#[derive(Debug)]
pub enum AppEvent {
    /// Fresh now-playing for every channel.
    NowPlaying(Vec<NowPlaying>),
    /// Schedule from the WebSocket, used to know when to refresh next.
    Schedule(Vec<ScheduleEntry>),
    /// Encoded image bytes for a show's cover art.
    Artwork {
        show_id: u64,
        bytes: Vec<u8>,
    },
    /// ICY `StreamTitle` straight off the audio stream.
    IcyTitle(String),
    /// Current audio amplitude on a 0.0..=1.0 scale, for the visualizer.
    Level(f64),
    PlayerState(PlayerState),
    /// Recoverable problem worth surfacing as a toast.
    Error(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::model::Mix;

    fn mix(genre: &[&str]) -> Mix {
        Mix {
            id: 1,
            title: "Foundation - Jul 20, 2026 - Framewerk".into(),
            artist_id: None,
            genre: genre.iter().map(|g| g.to_string()).collect(),
            track_list: vec![],
            show_id: None,
        }
    }

    fn now_playing(artist: Option<&str>, genre: &[&str]) -> NowPlaying {
        NowPlaying {
            channel: Channel::Frisky,
            mix: mix(genre),
            artist: artist.map(str::to_owned),
            show_id: None,
        }
    }

    #[test]
    fn subtitle_combines_artist_and_genre() {
        assert_eq!(
            now_playing(Some("Framewerk"), &["Breaks"]).subtitle(),
            "Framewerk · Breaks"
        );
    }

    #[test]
    fn subtitle_degrades_when_parts_are_missing() {
        assert_eq!(now_playing(Some("Framewerk"), &[]).subtitle(), "Framewerk");
        assert_eq!(now_playing(None, &["Breaks"]).subtitle(), "Breaks");
        // With nothing at all, name the channel rather than showing an empty line.
        assert_eq!(now_playing(None, &[]).subtitle(), "Frisky");
    }

    #[test]
    fn multiple_genres_are_joined() {
        assert_eq!(
            now_playing(None, &["Breaks", "Techno"]).subtitle(),
            "Breaks, Techno"
        );
    }
}
