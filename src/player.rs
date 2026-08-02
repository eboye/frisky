//! GStreamer playback for the live radio streams.
//!
//! Live radio has no seekable timeline, so "pause" is meaningless: pausing a
//! network stream just accumulates stale audio that plays back late. The
//! transport is therefore play/stop, tearing the pipeline down to `Null` when
//! stopped. MPRIS still exposes Pause and maps it here.

use gst::prelude::*;
use gstreamer as gst;
use gtk::glib;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use tracing::{debug, warn};

use crate::channel::{Channel, Quality};
use crate::event::{AppEvent, PlayerState, Sender};

/// Matches the web player, which gives up after four attempts and reports the
/// stream as busy.
const MAX_RETRIES: u32 = 4;
const RETRY_DELAY: Duration = Duration::from_secs(2);

/// How often the `level` element reports amplitude. 50 ms gives the visualiser
/// 20 samples a second — enough to look alive without flooding the bus.
const LEVEL_INTERVAL: Duration = Duration::from_millis(50);

/// Amplitudes below this many dB are treated as silence. Broadcast audio rarely
/// dips under it, so it sets the visible floor of the visualiser.
const LEVEL_FLOOR_DB: f64 = -60.0;

pub struct Player {
    playbin: gst::Element,
    events: Sender,
    /// What we are meant to be playing, so a retry can rebuild it.
    current: RefCell<Option<(Channel, Quality, Option<String>)>>,
    retries: Cell<u32>,
    state: Cell<PlayerState>,
    volume: Cell<f64>,
    /// Dropping this removes the bus watch, so it must outlive the player.
    bus_watch: RefCell<Option<gst::bus::BusWatchGuard>>,
    /// Logged once per stream so a silent visualiser is diagnosable without
    /// drowning the log in twenty messages a second.
    level_seen: Cell<bool>,
}

impl Player {
    /// Builds the pipeline and starts watching its bus.
    ///
    /// Must be called on the main thread: the bus watch dispatches on the
    /// thread-default main context.
    pub fn new(events: Sender) -> Result<Rc<Self>, anyhow::Error> {
        gst::init()?;

        // playbin3 handles the ICY metadata demux and MP3 decode selection for
        // us; every plugin it needs is present in the GNOME runtime.
        let playbin = gst::ElementFactory::make("playbin3")
            .build()
            .or_else(|_| gst::ElementFactory::make("playbin").build())?;

        // `level` posts RMS amplitude on the bus, which drives the visualiser.
        // It is an analyser, not a transform, so it passes audio through
        // untouched; if it is somehow unavailable, playback carries on without
        // a visualiser.
        match gst::ElementFactory::make("level")
            .property("post-messages", true)
            .property("interval", LEVEL_INTERVAL.as_nanos() as u64)
            .build()
        {
            Ok(level) => playbin.set_property("audio-filter", &level),
            Err(error) => warn!("visualiser unavailable ({error}); playing without it"),
        }

        let player = Rc::new(Self {
            playbin,
            events,
            current: RefCell::new(None),
            retries: Cell::new(0),
            state: Cell::new(PlayerState::Stopped),
            volume: Cell::new(1.0),
            bus_watch: RefCell::new(None),
            level_seen: Cell::new(false),
        });

        player.watch_bus()?;
        Ok(player)
    }

    pub fn state(&self) -> PlayerState {
        self.state.get()
    }

    pub fn is_active(&self) -> bool {
        !matches!(self.state.get(), PlayerState::Stopped)
    }

    pub fn current_channel(&self) -> Option<Channel> {
        self.current.borrow().as_ref().map(|(c, _, _)| *c)
    }

    /// Starts (or restarts) playback of a channel.
    pub fn play(&self, channel: Channel, quality: Quality, token: Option<String>) {
        self.retries.set(0);
        self.level_seen.set(false);
        *self.current.borrow_mut() = Some((channel, quality, token));
        self.start();
    }

    fn start(&self) {
        let Some((channel, quality, token)) = self.current.borrow().clone() else {
            return;
        };
        let uri = channel.stream_url(quality, token.as_deref());
        debug!("starting playback: {}", redact_token(&uri));

        // The URI can only be changed from Null.
        let _ = self.playbin.set_state(gst::State::Null);
        self.playbin.set_property("uri", &uri);
        self.apply_volume();

        self.set_state(PlayerState::Buffering);

        if let Err(error) = self.playbin.set_state(gst::State::Playing) {
            warn!("failed to start pipeline: {error}");
            self.emit(AppEvent::Error("Could not start playback.".into()));
            self.stop();
        }
    }

    pub fn stop(&self) {
        let _ = self.playbin.set_state(gst::State::Null);
        self.retries.set(0);
        self.set_state(PlayerState::Stopped);
        // No more level messages will arrive; let the visualiser settle to
        // silence rather than freezing on the last waveform.
        self.emit(AppEvent::Level(0.0));
    }

    /// Volume on a 0.0..=1.0 perceptual scale.
    ///
    /// playbin's `volume` is a linear gain, which sounds top-heavy on a slider,
    /// so map it cubically the way GStreamer's own volume helpers do.
    pub fn set_volume(&self, volume: f64) {
        self.volume.set(volume.clamp(0.0, 1.0));
        self.apply_volume();
    }

    pub fn volume(&self) -> f64 {
        self.volume.get()
    }

    fn apply_volume(&self) {
        let cubic = self.volume.get();
        self.playbin.set_property("volume", cubic * cubic * cubic);
    }

    fn set_state(&self, state: PlayerState) {
        if self.state.get() != state {
            self.state.set(state);
            self.emit(AppEvent::PlayerState(state));
        }
    }

    fn emit(&self, event: AppEvent) {
        // Unbounded channel; the only failure is a closed receiver at shutdown.
        let _ = self.events.send_blocking(event);
    }

    fn watch_bus(self: &Rc<Self>) -> Result<(), anyhow::Error> {
        let bus = self
            .playbin
            .bus()
            .ok_or_else(|| anyhow::anyhow!("pipeline has no bus"))?;

        // Weak, so the watch does not keep the player alive forever.
        let player = Rc::downgrade(self);
        let guard = bus.add_watch_local(move |_, message| {
            let Some(player) = player.upgrade() else {
                return glib::ControlFlow::Break;
            };
            player.handle_message(message);
            glib::ControlFlow::Continue
        })?;

        *self.bus_watch.borrow_mut() = Some(guard);
        Ok(())
    }

    fn handle_message(self: &Rc<Self>, message: &gst::Message) {
        use gst::MessageView;

        match message.view() {
            MessageView::Tag(tag) => {
                if let Some(title) = tag.tags().get::<gst::tags::Title>() {
                    let raw = title.get();
                    if let Some(parsed) = parse_icy_title(raw) {
                        self.emit(AppEvent::IcyTitle(parsed));
                    }
                }
            }
            MessageView::Buffering(buffering) => {
                let percent = buffering.percent();
                if percent < 100 {
                    self.set_state(PlayerState::Buffering);
                } else if self.state.get() == PlayerState::Buffering {
                    self.set_state(PlayerState::Playing);
                }
            }
            MessageView::StateChanged(changed) => {
                // Only the pipeline's own transitions matter.
                if changed.src() != Some(self.playbin.upcast_ref()) {
                    return;
                }
                match changed.current() {
                    gst::State::Playing => {
                        self.retries.set(0);
                        self.set_state(PlayerState::Playing);
                    }
                    gst::State::Null => self.set_state(PlayerState::Stopped),
                    _ => {}
                }
            }
            MessageView::Element(element) => {
                if let Some(level) = element
                    .structure()
                    .filter(|s| s.name() == "level")
                    .and_then(peak_amplitude)
                {
                    if !self.level_seen.replace(true) {
                        debug!("visualiser receiving audio levels");
                    }
                    self.emit(AppEvent::Level(level));
                }
            }
            MessageView::Error(error) => {
                warn!(
                    "pipeline error from {:?}: {} ({:?})",
                    error.src().map(|s| s.path_string()),
                    error.error(),
                    error.debug()
                );
                self.retry_or_fail();
            }
            MessageView::Eos(_) => {
                // A live stream should never end; treat it as a dropped
                // connection and reconnect.
                debug!("unexpected end of stream, reconnecting");
                self.retry_or_fail();
            }
            _ => {}
        }
    }

    /// Reconnects after a stream failure, giving up once the attempts are spent.
    fn retry_or_fail(self: &Rc<Self>) {
        let attempt = self.retries.get() + 1;
        let _ = self.playbin.set_state(gst::State::Null);

        if attempt > MAX_RETRIES {
            self.emit(AppEvent::Error(
                "Stream unavailable. The server may be busy — try again shortly.".into(),
            ));
            self.stop();
            return;
        }

        self.retries.set(attempt);
        self.set_state(PlayerState::Buffering);
        debug!("retrying stream ({attempt}/{MAX_RETRIES})");

        let player = Rc::downgrade(self);
        glib::timeout_add_local_once(RETRY_DELAY, move || {
            if let Some(player) = player.upgrade() {
                // A stop or channel change during the delay wins.
                if player.state.get() == PlayerState::Buffering {
                    player.start();
                }
            }
        });
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.playbin.set_state(gst::State::Null);
    }
}

/// Extracts something worth displaying from an ICY `StreamTitle`.
///
/// FRISKY sends `FRISKY | Foundation - Framewerk | for tracklist and more:
/// FRISKY.fm` — a station tag, the actual programme, then a promo. Keep the
/// middle. If the shape ever changes, fall back to the raw string so the UI
/// degrades instead of going blank.
fn parse_icy_title(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let meaningful: Vec<&str> = raw
        .split('|')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .filter(|part| !part.eq_ignore_ascii_case("frisky"))
        .filter(|part| !part.to_ascii_lowercase().contains("frisky.fm"))
        .collect();

    if meaningful.is_empty() {
        return Some(raw.to_owned());
    }
    Some(meaningful.join(" — "))
}

/// Loudest channel of a `level` message, as a 0.0..=1.0 amplitude.
///
/// The element reports RMS in dBFS per channel. Taking the maximum across
/// channels means a hard-panned sound still registers, and mapping the dB
/// range onto a linear scale keeps quiet passages visible — a raw
/// `10^(dB/20)` conversion would leave most of the waveform flat.
fn peak_amplitude(structure: &gst::StructureRef) -> Option<f64> {
    let loudest = loudest_channel_db(structure)?;
    Some(normalise_db(loudest))
}

/// Reads the loudest per-channel RMS value, in dBFS.
///
/// `level` sends `rms` as a `GValueArray`, but `GstValueList` and `GstArray`
/// both appear in the wild depending on element and version, so try each rather
/// than silently returning nothing — a wrong guess here shows up only as a
/// visualiser that never moves.
fn loudest_channel_db(structure: &gst::StructureRef) -> Option<f64> {
    let decibels: Vec<f64> = if let Ok(array) = structure.get::<glib::ValueArray>("rms") {
        array
            .as_slice()
            .iter()
            .filter_map(|value| value.get::<f64>().ok())
            .collect()
    } else if let Ok(list) = structure.get::<gst::List>("rms") {
        list.iter()
            .filter_map(|value| value.get::<f64>().ok())
            .collect()
    } else if let Ok(array) = structure.get::<gst::Array>("rms") {
        array
            .iter()
            .filter_map(|value| value.get::<f64>().ok())
            .collect()
    } else {
        return None;
    };

    decibels
        .into_iter()
        .filter(|db| db.is_finite())
        .fold(None, |peak: Option<f64>, db| {
            Some(peak.map_or(db, |peak| peak.max(db)))
        })
}

/// Maps dBFS onto 0.0..=1.0, with [`LEVEL_FLOOR_DB`] as silence.
fn normalise_db(db: f64) -> f64 {
    ((db - LEVEL_FLOOR_DB) / -LEVEL_FLOOR_DB).clamp(0.0, 1.0)
}

/// Keeps subscriber tokens out of the logs.
fn redact_token(uri: &str) -> String {
    match uri.split_once("?token=") {
        Some((base, _)) => format!("{base}?token=<redacted>"),
        None => uri.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_station_tag_and_promo() {
        assert_eq!(
            parse_icy_title("FRISKY | Foundation - Framewerk | for tracklist and more: FRISKY.fm")
                .as_deref(),
            Some("Foundation - Framewerk")
        );
    }

    #[test]
    fn handles_titles_without_a_promo_segment() {
        assert_eq!(
            parse_icy_title("FRISKY | The Shaka Series - August 2024 - LADS").as_deref(),
            Some("The Shaka Series - August 2024 - LADS")
        );
    }

    #[test]
    fn keeps_multiple_meaningful_segments() {
        assert_eq!(
            parse_icy_title("FRISKY | Show Name | Artist Name").as_deref(),
            Some("Show Name — Artist Name")
        );
    }

    #[test]
    fn falls_back_to_raw_when_nothing_survives_filtering() {
        // Every segment is filtered out, but showing the station beats blank.
        assert_eq!(parse_icy_title("FRISKY").as_deref(), Some("FRISKY"));
        assert_eq!(
            parse_icy_title("FRISKY | FRISKY.fm").as_deref(),
            Some("FRISKY | FRISKY.fm")
        );
    }

    #[test]
    fn unstructured_titles_pass_through() {
        assert_eq!(
            parse_icy_title("Some Artist - Some Track").as_deref(),
            Some("Some Artist - Some Track")
        );
    }

    #[test]
    fn empty_titles_are_ignored() {
        assert_eq!(parse_icy_title(""), None);
        assert_eq!(parse_icy_title("   "), None);
    }

    /// Runs real audio through a real `level` element.
    ///
    /// The wire format is the whole risk here: `rms` arrives as a
    /// `GValueArray`, which cannot be constructed into a `gst::Structure` from
    /// safe Rust (it is not `Send`) nor round-tripped through GStreamer's
    /// serialisation. Reading it as the wrong type fails silently and shows up
    /// only as a visualiser that never moves — so drive the actual element.
    fn capture_level_structure(volume: f64) -> Option<gst::Structure> {
        gst::init().unwrap();

        let pipeline = gst::parse::launch(&format!(
            "audiotestsrc num-buffers=20 volume={volume} ! audioconvert ! \
             level interval=10000000 post-messages=true ! fakesink sync=false"
        ))
        .expect("test pipeline should build");

        pipeline.set_state(gst::State::Playing).unwrap();
        let bus = pipeline.bus().unwrap();

        let mut captured = None;
        while let Some(message) = bus.timed_pop(Some(gst::ClockTime::from_seconds(5))) {
            match message.view() {
                gst::MessageView::Element(element) => {
                    if let Some(structure) = element.structure().filter(|s| s.name() == "level") {
                        captured = Some(structure.to_owned());
                        break;
                    }
                }
                gst::MessageView::Eos(_) | gst::MessageView::Error(_) => break,
                _ => {}
            }
        }

        pipeline.set_state(gst::State::Null).unwrap();
        captured
    }

    #[test]
    fn reads_rms_from_a_real_level_element() {
        let structure = capture_level_structure(0.8).expect("level should post a message");

        let db = loudest_channel_db(&structure).expect("rms should be readable");
        assert!(db.is_finite(), "expected a finite dBFS reading, got {db}");
        assert!(db <= 0.0, "dBFS should not exceed full scale, got {db}");

        let amplitude = peak_amplitude(&structure).expect("amplitude should be derivable");
        assert!(
            (0.0..=1.0).contains(&amplitude),
            "amplitude out of range: {amplitude}"
        );
        // A tone at 0.8 volume is nowhere near silence.
        assert!(amplitude > 0.5, "expected a loud reading, got {amplitude}");
    }

    #[test]
    fn silence_reads_as_no_amplitude_or_zero() {
        let structure = capture_level_structure(0.0).expect("level should post a message");

        // Digital silence is reported as -inf, which must not survive as a peak.
        match peak_amplitude(&structure) {
            None => {}
            Some(amplitude) => assert!(
                amplitude < 0.05,
                "silence should read as near-zero, got {amplitude}"
            ),
        }
    }

    #[test]
    fn ignores_level_messages_without_usable_rms() {
        gst::init().unwrap();
        let missing = gst::Structure::builder("level").build();
        assert_eq!(loudest_channel_db(&missing), None);
        assert_eq!(peak_amplitude(&missing), None);
    }

    #[test]
    fn normalises_the_db_range_onto_the_unit_interval() {
        assert_eq!(normalise_db(0.0), 1.0);
        assert_eq!(normalise_db(LEVEL_FLOOR_DB), 0.0);
        assert!((normalise_db(-30.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clamps_levels_outside_the_expected_range() {
        // The element can report just above 0 dB on clipping, and far below the
        // floor during silence.
        assert_eq!(normalise_db(6.0), 1.0);
        assert_eq!(normalise_db(-120.0), 0.0);
        assert_eq!(normalise_db(f64::NEG_INFINITY), 0.0);
    }

    #[test]
    fn redacts_tokens_from_logged_uris() {
        assert_eq!(
            redact_token("https://stream.deep.friskyradio.com/mp3_high?token=secret"),
            "https://stream.deep.friskyradio.com/mp3_high?token=<redacted>"
        );
        assert_eq!(
            redact_token("https://stream.deep.friskyradio.com/mp3_low"),
            "https://stream.deep.friskyradio.com/mp3_low"
        );
    }
}
