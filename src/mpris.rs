//! MPRIS integration: GNOME's top-bar media control, media keys, and lock
//! screen.
//!
//! Live radio has no timeline, so `Position` is always zero and `CanSeek` is
//! false. Pause is still advertised, because desktops expect a pause control
//! and media keys emit `PlayPause` — it maps to stopping the pipeline, which
//! for a live stream is the only sensible reading of "pause".
//!
//! Next/Previous move between channels, which is the closest analogue radio has
//! to tracks.

use adw::prelude::*;
use gtk::glib;
use mpris_server::zbus::fdo;
use mpris_server::{
    LocalPlayerInterface, LocalRootInterface, LocalServer, Metadata, PlaybackRate, PlaybackStatus,
    Property, Time, TrackId, Volume,
};
use std::cell::RefCell;
use std::rc::Rc;
use tracing::{debug, warn};

use crate::app::{FriskyApplication, APP_ID};
use crate::channel::Channel;
use crate::event::PlayerState;
use crate::player::Player;
use crate::window::FriskyWindow;

/// Bus name becomes `org.mpris.MediaPlayer2.friskygtk`.
const BUS_SUFFIX: &str = "friskygtk";

/// A stand-in track path. The spec requires a valid object path, and live radio
/// has no per-track identity to encode.
const TRACK_PATH: &str = "/io/github/eboye/Frisky/CurrentMix";

thread_local! {
    static SERVER: RefCell<Option<Rc<LocalServer<FriskyMpris>>>> =
        const { RefCell::new(None) };
}

/// Publishes the MPRIS interface for this window.
pub fn attach(app: &FriskyApplication, window: &FriskyWindow, player: Rc<Player>) {
    let imp = FriskyMpris {
        app: app.downgrade(),
        window: window.downgrade(),
        player,
    };

    glib::spawn_future_local(async move {
        match LocalServer::new(BUS_SUFFIX, imp).await {
            Ok(server) => {
                let server = Rc::new(server);
                // The server only handles calls while this task is polled.
                glib::spawn_future_local(server.run());
                SERVER.with(|slot| *slot.borrow_mut() = Some(server));
                debug!("MPRIS interface published");
            }
            // Losing MPRIS costs the top-bar control, not playback.
            Err(error) => warn!("could not publish MPRIS interface: {error}"),
        }
    });
}

/// Tells listeners that playback state or track metadata changed.
pub fn notify_changed() {
    let Some(server) = SERVER.with(|slot| slot.borrow().clone()) else {
        return;
    };
    glib::spawn_future_local(async move {
        let changed = server
            .properties_changed([
                Property::PlaybackStatus(server.imp().playback_status_now()),
                Property::Metadata(server.imp().metadata_now()),
                Property::CanPause(server.imp().is_active()),
            ])
            .await;
        if let Err(error) = changed {
            debug!("MPRIS property update failed: {error}");
        }
    });
}

/// Station name for the MPRIS `xesam:album` field.
///
/// The flagship channel shares its name with the station, so the usual
/// "FRISKY <channel>" form would read "FRISKY Frisky".
fn album_name(channel: Channel) -> String {
    match channel {
        Channel::Frisky => "FRISKY".to_owned(),
        other => format!("FRISKY {}", other.title()),
    }
}

pub struct FriskyMpris {
    app: glib::WeakRef<FriskyApplication>,
    window: glib::WeakRef<FriskyWindow>,
    player: Rc<Player>,
}

impl FriskyMpris {
    fn window(&self) -> Option<FriskyWindow> {
        self.window.upgrade()
    }

    fn is_active(&self) -> bool {
        self.player.is_active()
    }

    fn playback_status_now(&self) -> PlaybackStatus {
        match self.player.state() {
            // Buffering is still "playing" as far as the desktop is concerned;
            // there is no separate MPRIS state for it.
            PlayerState::Playing | PlayerState::Buffering => PlaybackStatus::Playing,
            PlayerState::Stopped => PlaybackStatus::Stopped,
        }
    }

    fn metadata_now(&self) -> Metadata {
        let mut builder =
            Metadata::builder().trackid(TrackId::try_from(TRACK_PATH).unwrap_or(TrackId::NO_TRACK));

        let Some(window) = self.window() else {
            return builder.build();
        };
        let channel = window.selected_channel();

        match window.current() {
            Some(entry) => {
                builder = builder
                    .title(entry.display_title())
                    .album(album_name(channel));

                if let Some(artist) = entry.artist.clone() {
                    builder = builder.artist([artist]);
                }
                if !entry.mix.genre.is_empty() {
                    builder = builder.genre(entry.mix.genre.clone());
                }
                if let Some(uri) = entry.show_id.and_then(crate::artwork::cached_uri) {
                    builder = builder.art_url(uri);
                }
            }
            None => builder = builder.title(channel.title()),
        }

        // Zero length marks an open-ended live stream.
        builder.length(Time::ZERO).build()
    }

    /// Moves `offset` channels from the current one, wrapping around.
    fn step_channel(&self, offset: isize) -> fdo::Result<()> {
        let Some(window) = self.window() else {
            return Ok(());
        };
        let current = window.selected_channel();
        let count = Channel::ALL.len() as isize;
        let index = Channel::ALL.iter().position(|c| *c == current).unwrap_or(0) as isize;

        let next = Channel::ALL[((index + offset).rem_euclid(count)) as usize];
        window.activate_channel(next);
        Ok(())
    }
}

impl LocalRootInterface for FriskyMpris {
    async fn raise(&self) -> fdo::Result<()> {
        if let Some(window) = self.window() {
            window.present();
        }
        Ok(())
    }

    async fn quit(&self) -> fdo::Result<()> {
        if let Some(app) = self.app.upgrade() {
            app.quit();
        }
        Ok(())
    }

    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_fullscreen(&self, _fullscreen: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn identity(&self) -> fdo::Result<String> {
        Ok("Frisky".to_owned())
    }

    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok(APP_ID.to_owned())
    }

    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        // The app plays fixed channels, not arbitrary URIs.
        Ok(vec![])
    }

    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec!["audio/mpeg".to_owned()])
    }
}

impl LocalPlayerInterface for FriskyMpris {
    async fn next(&self) -> fdo::Result<()> {
        self.step_channel(1)
    }

    async fn previous(&self) -> fdo::Result<()> {
        self.step_channel(-1)
    }

    async fn pause(&self) -> fdo::Result<()> {
        // Stop rather than pause: a paused live stream would resume minutes
        // behind live.
        self.player.stop();
        Ok(())
    }

    async fn play_pause(&self) -> fdo::Result<()> {
        if let Some(window) = self.window() {
            window.toggle_playback_external();
        }
        Ok(())
    }

    async fn stop(&self) -> fdo::Result<()> {
        self.player.stop();
        Ok(())
    }

    async fn play(&self) -> fdo::Result<()> {
        if let Some(window) = self.window() {
            window.start_playback_external();
        }
        Ok(())
    }

    async fn seek(&self, _offset: Time) -> fdo::Result<()> {
        Ok(())
    }

    async fn set_position(&self, _track_id: TrackId, _position: Time) -> fdo::Result<()> {
        Ok(())
    }

    async fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "Frisky plays its own channels only".into(),
        ))
    }

    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        Ok(self.playback_status_now())
    }

    async fn loop_status(&self) -> fdo::Result<mpris_server::LoopStatus> {
        Ok(mpris_server::LoopStatus::None)
    }

    async fn set_loop_status(
        &self,
        _loop_status: mpris_server::LoopStatus,
    ) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn set_rate(&self, _rate: PlaybackRate) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn set_shuffle(&self, _shuffle: bool) -> mpris_server::zbus::Result<()> {
        Ok(())
    }

    async fn metadata(&self) -> fdo::Result<Metadata> {
        Ok(self.metadata_now())
    }

    async fn volume(&self) -> fdo::Result<Volume> {
        Ok(self.player.volume())
    }

    async fn set_volume(&self, volume: Volume) -> mpris_server::zbus::Result<()> {
        if let Some(window) = self.window() {
            window.set_volume_external(volume);
        }
        Ok(())
    }

    async fn position(&self) -> fdo::Result<Time> {
        // Live stream: there is no meaningful position.
        Ok(Time::ZERO)
    }

    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn album_avoids_repeating_the_station_name() {
        assert_eq!(album_name(Channel::Frisky), "FRISKY");
        assert_eq!(album_name(Channel::Deep), "FRISKY Deep");
        assert_eq!(album_name(Channel::Chill), "FRISKY Chill");
        assert_eq!(album_name(Channel::Classics), "FRISKY Classics");
    }
}
