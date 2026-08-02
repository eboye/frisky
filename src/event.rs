//! The one-way channel from background work into the GTK main loop.
//!
//! Nothing in here may hold a GTK type: GDK/GTK objects are not `Send`, so
//! artwork travels as raw bytes and is decoded into a `gdk::Texture` on the
//! main thread.

use crate::api::model::{Mix, ScheduleEntry};
use crate::channel::Channel;
use chrono::{DateTime, Utc};

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

/// How far through the airing mix we are.
///
/// Live radio has no per-track position, but the schedule gives each mix a
/// start and an end, so progress through the *mix* is real information rather
/// than a guess.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixProgress {
    /// 0.0..=1.0 through the mix.
    pub fraction: f64,
    pub elapsed: i64,
    pub total: i64,
}

impl MixProgress {
    /// `MM:SS`, or `H:MM:SS` for mixes that run past an hour.
    pub fn format(seconds: i64) -> String {
        let seconds = seconds.max(0);
        let (hours, minutes, seconds) = (seconds / 3600, (seconds / 60) % 60, seconds % 60);
        if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        }
    }

    pub fn label(&self) -> String {
        format!(
            "{} / {}",
            Self::format(self.elapsed),
            Self::format(self.total)
        )
    }
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
    /// Scheduled airing window, used for the progress bar.
    pub started_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
}

impl NowPlaying {
    /// Display title, preferring "Show - Artist" over the raw mix title, which
    /// carries a date that is noise in a now-playing line.
    pub fn display_title(&self) -> String {
        self.mix.title.clone()
    }

    /// Progress through the airing mix at `now`.
    ///
    /// `None` when the schedule did not carry a usable window, so callers can
    /// hide the progress bar rather than show a meaningless zero.
    pub fn progress_at(&self, now: DateTime<Utc>) -> Option<MixProgress> {
        let (start, end) = (self.started_at?, self.ends_at?);

        let total = (end - start).num_seconds();
        if total <= 0 {
            return None;
        }

        // Clamped at both ends: the stream can run slightly past its slot, and
        // a refresh can land just before one starts.
        let elapsed = (now - start).num_seconds().clamp(0, total);
        Some(MixProgress {
            fraction: elapsed as f64 / total as f64,
            elapsed,
            total,
        })
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
            started_at: None,
            ends_at: None,
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

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// A one-hour mix, 11:00 to 12:00.
    fn scheduled() -> NowPlaying {
        let mut entry = now_playing(None, &[]);
        entry.started_at = Some(at("2026-08-02T11:00:00Z"));
        entry.ends_at = Some(at("2026-08-02T12:00:00Z"));
        entry
    }

    #[test]
    fn progress_tracks_position_through_the_mix() {
        let progress = scheduled()
            .progress_at(at("2026-08-02T11:30:00Z"))
            .expect("a scheduled mix has progress");

        assert!((progress.fraction - 0.5).abs() < 1e-9);
        assert_eq!(progress.elapsed, 1800);
        assert_eq!(progress.total, 3600);
    }

    #[test]
    fn progress_is_clamped_outside_the_scheduled_window() {
        // A refresh landing just before the slot starts, and a stream running
        // past its end, must both stay on the bar.
        let early = scheduled().progress_at(at("2026-08-02T10:45:00Z")).unwrap();
        assert_eq!(early.fraction, 0.0);
        assert_eq!(early.elapsed, 0);

        let late = scheduled().progress_at(at("2026-08-02T12:30:00Z")).unwrap();
        assert_eq!(late.fraction, 1.0);
        assert_eq!(late.elapsed, 3600);
    }

    #[test]
    fn progress_is_absent_without_a_usable_window() {
        // Nothing scheduled at all.
        assert_eq!(
            now_playing(None, &[]).progress_at(at("2026-08-02T11:30:00Z")),
            None
        );

        // Only one end of the window.
        let mut half = now_playing(None, &[]);
        half.started_at = Some(at("2026-08-02T11:00:00Z"));
        assert_eq!(half.progress_at(at("2026-08-02T11:30:00Z")), None);

        // A zero-length slot would divide by zero.
        let mut instant = now_playing(None, &[]);
        instant.started_at = Some(at("2026-08-02T11:00:00Z"));
        instant.ends_at = Some(at("2026-08-02T11:00:00Z"));
        assert_eq!(instant.progress_at(at("2026-08-02T11:00:00Z")), None);
    }

    #[test]
    fn durations_format_as_clock_times() {
        assert_eq!(MixProgress::format(0), "0:00");
        assert_eq!(MixProgress::format(59), "0:59");
        assert_eq!(MixProgress::format(605), "10:05");
        // Two-hour mixes are common, so hours must not be folded into minutes.
        assert_eq!(MixProgress::format(3600), "1:00:00");
        assert_eq!(MixProgress::format(7325), "2:02:05");
        // Never render a negative clock.
        assert_eq!(MixProgress::format(-5), "0:00");
    }

    #[test]
    fn progress_label_reads_as_elapsed_over_total() {
        let progress = scheduled().progress_at(at("2026-08-02T11:12:34Z")).unwrap();
        assert_eq!(progress.label(), "12:34 / 1:00:00");
    }
}
