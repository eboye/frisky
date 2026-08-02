//! Serde models for the `api.frisky.fm/v3` responses the app consumes.
//!
//! These mirror the live JSON rather than an published schema, so every field
//! the app does not strictly need is optional. A shape change upstream should
//! degrade the UI, not fail the whole deserialization.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::collections::HashMap;

/// A `{ "id": 123, "model": "Mixes", "link": "v3/mixes/123" }` reference.
///
/// The API uses these in place of embedded objects; the id is the only part
/// worth keeping.
#[derive(Debug, Clone, Deserialize)]
pub struct Ref {
    pub id: u64,
}

/// An uploaded image. Only `url` is required; the API omits the rest on some
/// records.
#[derive(Debug, Clone, Deserialize)]
pub struct Image {
    pub url: String,
}

/// Response body of `GET /v3/stations`, keyed by station id.
pub type StationsResponse = HashMap<String, Station>;

/// Deserializes `T`, degrading to `None` rather than failing.
///
/// `GET /v3/stations` returns all four channels in one object, so serde's
/// all-or-nothing parsing means one station growing an unexpected field type
/// would blank the entire app. Isolating the failure costs one intermediate
/// `Value` per station and keeps the other three playing.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

#[derive(Debug, Clone, Deserialize)]
pub struct Station {
    /// Not read: the response is keyed by station id already. Kept optional so
    /// its absence can never fail the parse.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub title: String,
    /// Currently-airing mix, already resolved by the API.
    #[serde(default, deserialize_with = "lenient")]
    pub mix: Option<Mix>,
    #[serde(rename = "scheduledMix", default, deserialize_with = "lenient")]
    pub scheduled_mix: Option<ScheduleEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mix {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist_id: Option<Ref>,
    #[serde(default)]
    pub genre: Vec<String>,
    #[serde(default)]
    pub track_list: Vec<Track>,
    #[serde(default)]
    pub show_id: Option<Ref>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
}

/// One entry of the schedule, as delivered by both
/// `GET /v3/stations/playlists` and the now-playing WebSocket.
#[derive(Debug, Clone, Deserialize)]
pub struct ScheduleEntry {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub mixes_id: Option<Ref>,
    #[serde(default)]
    pub station: String,
    #[serde(default)]
    pub scheduled_start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scheduled_end_time: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Show {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
    /// Square 1200x1200 artwork. Preferred for cover display.
    #[serde(default)]
    pub album_art: Option<Image>,
    /// Wide banner. Used only if `album_art` is missing.
    #[serde(default)]
    pub image: Option<Image>,
}

impl Show {
    /// Best available artwork URL, preferring the square album art.
    pub fn artwork_url(&self) -> Option<&str> {
        self.album_art
            .as_ref()
            .or(self.image.as_ref())
            .map(|i| i.url.as_str())
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artist {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
}

/// Response of `GET /v3/subscriptions/validate-streaming`.
#[derive(Debug, Clone, Deserialize)]
pub struct StreamValidation {
    #[serde(default)]
    pub allowed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trimmed from a real GET /v3/stations response.
    const STATIONS_JSON: &str = r#"{
      "frisky": {
        "id": "frisky",
        "serversId": 5,
        "title": "Frisky",
        "servers": {"items": {"mp3_low": {"title": "96K MP3", "urls": ["https://stream.frisky.friskyradio.com/mp3_low"]}}},
        "scheduledMix": {
          "id": 239568, "path": "foundation/JPYZ2C.mp3",
          "mixes_id": {"id": 73573, "model": "Mixes", "link": "v3/mixes/73573"},
          "station": "frisky", "duration": 3587, "premiere": 0, "fixed": 0,
          "skip_time": null,
          "scheduled_start_time": "2026-08-02T11:22:20.000Z",
          "scheduled_end_time": "2026-08-02T12:22:07.000Z",
          "air_start_time": null, "air_end_time": null
        },
        "mix": {
          "id": 73573, "title": "Foundation - Jul 20, 2026 - Framewerk",
          "url": "framewerk-at-07-20-2026",
          "artist_id": {"id": 25198, "model": "Artists", "link": "v3/artists/25198"},
          "genre": ["Breaks"],
          "track_list": [{"title": "A Track", "artist": "An Artist"}],
          "show_id": {"id": 53592, "model": "Shows", "link": "v3/shows/53592"},
          "episode_id": {"id": 73574, "model": "Episodes", "link": "v3/episodes/73574"},
          "allow_playing": 1, "reach": 1546, "favorite_count": 9
        }
      }
    }"#;

    #[test]
    fn parses_stations_response() {
        let stations: StationsResponse = serde_json::from_str(STATIONS_JSON).unwrap();
        let frisky = &stations["frisky"];
        assert_eq!(frisky.title, "Frisky");

        let mix = frisky.mix.as_ref().unwrap();
        assert_eq!(mix.id, 73573);
        assert_eq!(mix.show_id.as_ref().unwrap().id, 53592);
        assert_eq!(mix.artist_id.as_ref().unwrap().id, 25198);
        assert_eq!(mix.genre, ["Breaks"]);
        assert_eq!(mix.track_list.len(), 1);
    }

    #[test]
    fn a_malformed_station_does_not_take_down_the_others() {
        // The whole app reads from one object, so serde's all-or-nothing
        // parsing would otherwise turn one upstream change into four blank
        // channels.
        let json = r#"{
          "frisky": {"id": "frisky", "title": "Frisky",
                     "mix": {"id": 1, "title": "Good mix", "genre": [], "track_list": []}},
          "deep": {"id": "deep", "title": "Deep",
                   "mix": {"id": "not-a-number", "title": 42}}
        }"#;

        let stations: StationsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            stations["frisky"].mix.as_ref().map(|m| m.title.as_str()),
            Some("Good mix")
        );
        // The broken one loses its mix, and only its mix.
        assert!(stations["deep"].mix.is_none());
        assert_eq!(stations["deep"].title, "Deep");
    }

    #[test]
    fn a_station_stripped_to_nothing_still_parses() {
        // Every field the app does not strictly need is optional, so an
        // unrecognisable station degrades rather than failing the response.
        let stations: StationsResponse = serde_json::from_str(r#"{"chill": {}}"#).unwrap();
        assert!(stations["chill"].mix.is_none());
        assert!(stations["chill"].scheduled_mix.is_none());
    }

    #[test]
    fn websocket_schedule_array_parses() {
        // The socket sends a bare array of the same entries.
        let json = r#"[
          {"id": 239574, "path": "aug/GJnZdb.mp3",
           "mixes_id": {"id": 73589, "model": "Mixes", "link": "v3/mixes/73589"},
           "station": "chill", "duration": 7022, "premiere": 0, "fixed": 1,
           "skip_time": null,
           "scheduled_start_time": "2026-08-02T11:00:00.000Z",
           "scheduled_end_time": "2026-08-02T12:57:02.000Z",
           "air_start_time": null, "air_end_time": null}
        ]"#;
        let entries: Vec<ScheduleEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].station, "chill");
        assert_eq!(entries[0].mixes_id.as_ref().map(|r| r.id), Some(73589));
    }

    #[test]
    fn show_prefers_album_art_over_banner() {
        let show: Show = serde_json::from_str(
            r#"{"id": 1, "title": "S",
                "album_art": {"url": "https://example.invalid/square.png"},
                "image": {"url": "https://example.invalid/wide.png"}}"#,
        )
        .unwrap();
        assert_eq!(
            show.artwork_url(),
            Some("https://example.invalid/square.png")
        );

        let banner_only: Show = serde_json::from_str(
            r#"{"id": 1, "title": "S", "album_art": null,
                "image": {"url": "https://example.invalid/wide.png"}}"#,
        )
        .unwrap();
        assert_eq!(
            banner_only.artwork_url(),
            Some("https://example.invalid/wide.png")
        );

        let neither: Show = serde_json::from_str(r#"{"id": 1, "title": "S"}"#).unwrap();
        assert_eq!(neither.artwork_url(), None);
    }

    #[test]
    fn missing_optional_fields_do_not_fail() {
        // Episodes and some mixes return nulls where objects normally sit.
        let mix: Mix = serde_json::from_str(
            r#"{"id": 1, "title": "Bare mix", "genre": [], "track_list": []}"#,
        )
        .unwrap();
        assert!(mix.show_id.is_none());
        assert!(mix.artist_id.is_none());
        assert!(mix.track_list.is_empty());
    }

    #[test]
    fn validation_defaults_to_denied() {
        let denied: StreamValidation = serde_json::from_str(r#"{"allowed": false}"#).unwrap();
        assert!(!denied.allowed);
        // An unexpected body must not read as permission granted.
        let empty: StreamValidation = serde_json::from_str(r#"{}"#).unwrap();
        assert!(!empty.allowed);
    }
}
