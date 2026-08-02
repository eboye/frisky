//! Keeps now-playing metadata fresh for all four channels.
//!
//! The `wss://api.frisky.fm/v3/stations/nowplaying` socket pushes a schedule of
//! roughly ten upcoming entries per station (about fourteen hours ahead) once on
//! connect, and again whenever the schedule is revised. It is *not* a per-track
//! ticker, so the coordinator uses it to work out when the current mix ends and
//! then sleeps until that moment instead of polling.
//!
//! If the socket is unavailable the coordinator still works, falling back to a
//! periodic refresh.

use anyhow::Result;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

use super::model::{Mix, ScheduleEntry, StationsResponse};
use super::FriskyClient;
use crate::channel::Channel;
use crate::event::{AppEvent, NowPlaying, Sender};

pub const WEBSOCKET_URL: &str = "wss://api.frisky.fm/v3/stations/nowplaying";

/// Used when the schedule cannot tell us when to look again.
const FALLBACK_REFRESH: Duration = Duration::from_secs(300);
/// Guards against refetching in a tight loop around a boundary.
const MIN_REFRESH: Duration = Duration::from_secs(15);
/// Let the server switch over before asking what replaced the mix.
const BOUNDARY_GRACE: Duration = Duration::from_secs(5);
/// Longest we will ever sleep, even if the next boundary is hours away.
///
/// Mixes run to two hours and the schedule reaches half a day ahead, so this is
/// not the normal path — it is a safety net for a stale schedule while the
/// socket is down. Sleeping through to a distant boundary is otherwise correct
/// and costs nothing.
const MAX_SLEEP: Duration = Duration::from_secs(1800);

const RECONNECT_MIN: Duration = Duration::from_secs(2);
const RECONNECT_MAX: Duration = Duration::from_secs(120);

/// Reason the coordinator woke up.
enum Wake {
    /// New schedule arrived over the socket.
    Schedule(Vec<ScheduleEntry>),
    /// A mix boundary was reached, or the fallback timer elapsed.
    Timer,
    /// Something else asked for a refresh (startup, manual, stream hiccup).
    Requested,
}

/// Handle used by the UI to nudge the coordinator.
#[derive(Clone)]
pub struct RefreshHandle(mpsc::UnboundedSender<()>);

impl RefreshHandle {
    pub fn request(&self) {
        let _ = self.0.send(());
    }
}

/// Runs until the app exits. Owns all now-playing networking.
pub async fn run(client: FriskyClient, events: Sender) -> RefreshHandle {
    let (refresh_tx, refresh_rx) = mpsc::unbounded_channel();
    let handle = RefreshHandle(refresh_tx);

    let (schedule_tx, schedule_rx) = mpsc::unbounded_channel();
    tokio::spawn(socket_loop(schedule_tx));
    tokio::spawn(coordinator(client, events, schedule_rx, refresh_rx));

    handle
}

async fn coordinator(
    client: FriskyClient,
    events: Sender,
    mut schedule_rx: mpsc::UnboundedReceiver<Vec<ScheduleEntry>>,
    mut refresh_rx: mpsc::UnboundedReceiver<()>,
) {
    // Resolved artist and show names are stable, so cache them across refreshes
    // rather than refetching on every mix change.
    let mut artists: HashMap<u64, String> = HashMap::new();
    let mut schedule: Vec<ScheduleEntry> = Vec::new();
    let mut wake = Wake::Requested;

    loop {
        match wake {
            Wake::Schedule(ref entries) => {
                schedule = entries.clone();
                let _ = events.send(AppEvent::Schedule(schedule.clone())).await;
            }
            Wake::Timer | Wake::Requested => {}
        }

        match refresh(&client, &events, &mut artists).await {
            Ok(()) => {}
            Err(error) => {
                warn!("now-playing refresh failed: {error:#}");
                let _ = events
                    .send(AppEvent::Error(format!("Could not reach FRISKY: {error}")))
                    .await;
            }
        }

        let sleep_for = next_wake_delay(&schedule, Utc::now());
        debug!("next now-playing refresh in {:?}", sleep_for);

        wake = tokio::select! {
            entries = schedule_rx.recv() => match entries {
                Some(entries) => Wake::Schedule(entries),
                // Socket task is gone; keep running on the timer alone.
                None => { tokio::time::sleep(sleep_for).await; Wake::Timer }
            },
            _ = refresh_rx.recv() => Wake::Requested,
            _ = tokio::time::sleep(sleep_for) => Wake::Timer,
        };
    }
}

/// Fetches every channel's current mix and emits it.
async fn refresh(
    client: &FriskyClient,
    events: &Sender,
    artists: &mut HashMap<u64, String>,
) -> Result<()> {
    let stations: StationsResponse = client.stations().await?;

    let mut playing = Vec::new();
    for (id, station) in &stations {
        let (Some(channel), Some(mix)) = (Channel::from_id(id), station.mix.clone()) else {
            continue;
        };
        playing.push(build_now_playing(client, channel, mix, artists).await);
    }

    // Keep a stable order so the UI never reshuffles.
    playing.sort_by_key(|p| Channel::ALL.iter().position(|c| *c == p.channel));

    if !playing.is_empty() {
        let _ = events.send(AppEvent::NowPlaying(playing)).await;
    }
    Ok(())
}

async fn build_now_playing(
    client: &FriskyClient,
    channel: Channel,
    mix: Mix,
    artists: &mut HashMap<u64, String>,
) -> NowPlaying {
    let show_id = mix.show_id.as_ref().map(|r| r.id);

    let artist = match mix.artist_id.as_ref().map(|r| r.id) {
        Some(id) => match artists.get(&id) {
            Some(name) => Some(name.clone()),
            None => match client.artist(id).await {
                Ok(artist) => {
                    artists.insert(id, artist.title.clone());
                    Some(artist.title)
                }
                Err(error) => {
                    // A missing artist name is cosmetic; carry on without it.
                    debug!("artist {id} lookup failed: {error:#}");
                    None
                }
            },
        },
        None => None,
    };

    NowPlaying {
        channel,
        mix,
        artist,
        show_id,
    }
}

/// How long to wait before refreshing, based on the earliest mix boundary still
/// ahead of us.
///
/// Clamped at both ends: never hammer the API around a boundary, never sleep so
/// long that a schedule revision goes unnoticed.
fn next_wake_delay(schedule: &[ScheduleEntry], now: DateTime<Utc>) -> Duration {
    let next_boundary = schedule
        .iter()
        .filter_map(|entry| entry.scheduled_end_time)
        .filter(|end| *end > now)
        .min();

    match next_boundary {
        Some(end) => (end - now)
            .to_std()
            .map(|delay| (delay + BOUNDARY_GRACE).clamp(MIN_REFRESH, MAX_SLEEP))
            .unwrap_or(FALLBACK_REFRESH),
        // Nothing to aim at: check back on a fixed cadence instead.
        None => FALLBACK_REFRESH,
    }
}

/// Maintains the WebSocket, reconnecting with exponential backoff.
async fn socket_loop(schedule_tx: mpsc::UnboundedSender<Vec<ScheduleEntry>>) {
    let mut backoff = RECONNECT_MIN;

    loop {
        match consume_socket(&schedule_tx).await {
            Ok(()) => {
                debug!("now-playing socket closed cleanly, reconnecting");
                backoff = RECONNECT_MIN;
            }
            Err(error) => {
                warn!("now-playing socket error: {error:#}");
            }
        }

        // The coordinator's timer keeps metadata fresh meanwhile, so a socket
        // outage degrades latency rather than breaking the app.
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

async fn consume_socket(schedule_tx: &mpsc::UnboundedSender<Vec<ScheduleEntry>>) -> Result<()> {
    let (stream, _) = tokio_tungstenite::connect_async(WEBSOCKET_URL).await?;
    debug!("now-playing socket connected");
    let (_, mut read) = stream.split();

    while let Some(message) = read.next().await {
        let text = match message? {
            Message::Text(text) => text.to_string(),
            Message::Binary(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Message::Close(_) => break,
            // tungstenite answers pings for us.
            _ => continue,
        };

        match serde_json::from_str::<Vec<ScheduleEntry>>(&text) {
            Ok(entries) if !entries.is_empty() => {
                if schedule_tx.send(entries).is_err() {
                    return Ok(()); // Coordinator is gone; stop.
                }
            }
            Ok(_) => {}
            Err(error) => debug!("ignoring unrecognised socket payload: {error}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::model::Ref;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn entry(station: &str, start: &str, end: &str) -> ScheduleEntry {
        ScheduleEntry {
            id: 1,
            mixes_id: Ref { id: 1 },
            station: station.into(),
            scheduled_start_time: Some(at(start)),
            scheduled_end_time: Some(at(end)),
        }
    }

    #[test]
    fn waits_until_the_earliest_upcoming_boundary() {
        let schedule = vec![
            entry("frisky", "2026-08-02T11:00:00Z", "2026-08-02T12:00:00Z"),
            entry("deep", "2026-08-02T11:00:00Z", "2026-08-02T12:30:00Z"),
        ];
        // 10 minutes to the frisky boundary, plus the grace period.
        let delay = next_wake_delay(&schedule, at("2026-08-02T11:50:00Z"));
        assert_eq!(delay, Duration::from_secs(600) + BOUNDARY_GRACE);
    }

    #[test]
    fn ignores_boundaries_already_past() {
        let schedule = vec![
            entry("frisky", "2026-08-02T09:00:00Z", "2026-08-02T10:00:00Z"),
            entry("deep", "2026-08-02T11:00:00Z", "2026-08-02T12:00:00Z"),
        ];
        let delay = next_wake_delay(&schedule, at("2026-08-02T11:55:00Z"));
        assert_eq!(delay, Duration::from_secs(300) + BOUNDARY_GRACE);
    }

    #[test]
    fn clamps_to_a_floor_at_a_boundary() {
        // Without a floor, a boundary one second away would refetch repeatedly.
        let schedule = vec![entry(
            "frisky",
            "2026-08-02T11:00:00Z",
            "2026-08-02T12:00:00Z",
        )];
        let delay = next_wake_delay(&schedule, at("2026-08-02T11:59:59Z"));
        assert_eq!(delay, MIN_REFRESH);
    }

    #[test]
    fn sleeps_the_whole_way_to_a_boundary_under_the_ceiling() {
        // The point of using the schedule: wait for the actual mix change
        // rather than polling on a fixed cadence.
        let schedule = vec![entry(
            "frisky",
            "2026-08-02T11:00:00Z",
            "2026-08-02T11:25:00Z",
        )];
        let delay = next_wake_delay(&schedule, at("2026-08-02T11:05:00Z"));
        assert_eq!(delay, Duration::from_secs(1200) + BOUNDARY_GRACE);
        assert!(delay > FALLBACK_REFRESH, "should outlast the blind cadence");
    }

    #[test]
    fn clamps_to_a_ceiling_for_distant_boundaries() {
        // A two-hour mix still gets checked on periodically, so a schedule that
        // turns out to be wrong cannot leave stale metadata on screen for the
        // whole mix.
        let schedule = vec![entry(
            "frisky",
            "2026-08-02T11:00:00Z",
            "2026-08-02T13:00:00Z",
        )];
        let delay = next_wake_delay(&schedule, at("2026-08-02T11:05:00Z"));
        assert_eq!(delay, MAX_SLEEP);
    }

    #[test]
    fn falls_back_without_a_schedule() {
        assert_eq!(
            next_wake_delay(&[], at("2026-08-02T11:00:00Z")),
            FALLBACK_REFRESH
        );
    }

    #[test]
    fn entries_without_end_times_are_skipped() {
        let mut broken = entry("frisky", "2026-08-02T11:00:00Z", "2026-08-02T12:00:00Z");
        broken.scheduled_end_time = None;
        assert_eq!(
            next_wake_delay(&[broken], at("2026-08-02T11:00:00Z")),
            FALLBACK_REFRESH
        );
    }
}
