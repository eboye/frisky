//! The four FRISKY radio channels.
//!
//! Stream hosts and mount points were verified against the live service: the
//! bare host URL is an alias for `mp3_low` (96k) and is served without
//! authentication, while `mp3_mid` (128k) and `mp3_high` (320k) return 401
//! unless a subscriber token is appended as `?token=`.
//!
//! Gradient stops mirror frisky.fm's own per-channel styling so the app reads
//! as part of the same family.

use gtk::glib;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Channel {
    /// The flagship channel, and the one to fall back to.
    #[default]
    Frisky,
    Deep,
    Chill,
    Classics,
}

/// Stream bitrate tier. `Low` is free; the other two require a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    #[default]
    Low,
    High,
    HiFi,
}

impl Quality {
    /// Mount point on the streaming host.
    pub fn mount(self) -> &'static str {
        match self {
            Self::Low => "mp3_low",
            Self::High => "mp3_mid",
            Self::HiFi => "mp3_high",
        }
    }

    pub fn requires_subscription(self) -> bool {
        !matches!(self, Self::Low)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::High => "High",
            Self::HiFi => "HI-FI",
        }
    }

    pub fn bitrate(self) -> &'static str {
        match self {
            Self::Low => "96 kbps",
            Self::High => "128 kbps",
            Self::HiFi => "320 kbps",
        }
    }

    /// Round-trips through the GSettings enum nick.
    pub fn from_nick(nick: &str) -> Self {
        match nick {
            "high" => Self::High,
            "hifi" => Self::HiFi,
            _ => Self::Low,
        }
    }

    pub fn nick(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
            Self::HiFi => "hifi",
        }
    }

    pub const ALL: [Quality; 3] = [Quality::Low, Quality::High, Quality::HiFi];
}

impl Channel {
    pub const ALL: [Channel; 4] = [
        Channel::Frisky,
        Channel::Deep,
        Channel::Chill,
        Channel::Classics,
    ];

    /// Station id as used by the API (`/v3/stations` keys, WebSocket `station`
    /// field, and the streaming subdomain).
    pub fn id(self) -> &'static str {
        match self {
            Self::Frisky => "frisky",
            Self::Deep => "deep",
            Self::Chill => "chill",
            Self::Classics => "classics",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Frisky => "Frisky",
            Self::Deep => "Deep",
            Self::Chill => "Chill",
            Self::Classics => "Classics",
        }
    }

    /// Uppercase form used on the channel pills, matching the site's wordmark.
    pub fn wordmark(self) -> &'static str {
        match self {
            Self::Frisky => "FRISKY",
            Self::Deep => "DEEP",
            Self::Chill => "CHILL",
            Self::Classics => "CLASSICS",
        }
    }

    /// One- or two-letter form for the compact channel chips. Chill and
    /// Classics share an initial, so neither can be a single letter alone.
    pub fn short_label(self) -> &'static str {
        match self {
            Self::Frisky => "F",
            Self::Deep => "D",
            Self::Chill => "CH",
            Self::Classics => "CL",
        }
    }

    pub fn tagline(self) -> &'static str {
        match self {
            Self::Frisky => "The flagship channel",
            Self::Deep => "Deep and melodic",
            Self::Chill => "Chillout and downtempo",
            Self::Classics => "Mixes from the archives",
        }
    }

    /// CSS class applied to this channel's pill; paired with a gradient in
    /// `style.css`.
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Frisky => "channel-frisky",
            Self::Deep => "channel-deep",
            Self::Chill => "channel-chill",
            Self::Classics => "channel-classics",
        }
    }

    /// Stream URL for the given quality, with an optional subscriber token.
    ///
    /// Mirrors the web player, which plays `{server}{token ? "?token=" + token : ""}`.
    pub fn stream_url(self, quality: Quality, token: Option<&str>) -> String {
        let base = format!(
            "https://stream.{}.friskyradio.com/{}",
            self.id(),
            quality.mount()
        );
        match token {
            // Percent-encoded, so a token carrying a reserved character cannot
            // silently truncate the query or graft extra parameters onto it.
            // Ordinary tokens are entirely unreserved characters and pass
            // through unchanged. `validate_stream` encodes the same value via
            // reqwest, so the two must agree on the wire.
            Some(t) if !t.is_empty() => {
                format!("{base}?token={}", glib::Uri::escape_string(t, None, false))
            }
            _ => base,
        }
    }
}

impl fmt::Display for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.title())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_stream_url_has_no_token() {
        assert_eq!(
            Channel::Classics.stream_url(Quality::Low, None),
            "https://stream.classics.friskyradio.com/mp3_low"
        );
    }

    #[test]
    fn premium_stream_url_appends_token() {
        assert_eq!(
            Channel::Deep.stream_url(Quality::HiFi, Some("abc123")),
            "https://stream.deep.friskyradio.com/mp3_high?token=abc123"
        );
    }

    #[test]
    fn ordinary_tokens_survive_encoding_unchanged() {
        // Real tokens are built from unreserved characters, so encoding must not
        // disturb the URL the web player would have produced. The fixture below
        // covers that whole character class — letters, digits, '-', '_', '.' and
        // '~' — deliberately without imitating the shape of a JWT, which entropy
        // scanners flag on sight.
        const UNRESERVED: &str = "placeholder-token_1.2~ok";
        assert_eq!(
            Channel::Deep.stream_url(Quality::HiFi, Some(UNRESERVED)),
            "https://stream.deep.friskyradio.com/mp3_high?token=placeholder-token_1.2~ok"
        );
    }

    #[test]
    fn reserved_characters_in_a_token_cannot_escape_the_query() {
        // Without encoding, the '&' would start a second parameter and the '#'
        // would truncate the URL entirely.
        let url = Channel::Deep.stream_url(Quality::HiFi, Some("a&b=c#d e"));
        assert_eq!(
            url,
            "https://stream.deep.friskyradio.com/mp3_high?token=a%26b%3Dc%23d%20e"
        );
        assert_eq!(url.matches('?').count(), 1);
        assert!(!url.contains('#'), "fragment would truncate the stream URL");
    }

    #[test]
    fn empty_token_is_treated_as_absent() {
        assert_eq!(
            Channel::Frisky.stream_url(Quality::Low, Some("")),
            "https://stream.frisky.friskyradio.com/mp3_low"
        );
    }

    #[test]
    fn ids_round_trip() {
        for channel in Channel::ALL {
            assert_eq!(Channel::from_id(channel.id()), Some(channel));
        }
        assert_eq!(Channel::from_id("nope"), None);
    }

    #[test]
    fn short_labels_are_unique() {
        // Ambiguous chips would make the compact switcher unusable.
        let mut labels: Vec<&str> = Channel::ALL.iter().map(|c| c.short_label()).collect();
        labels.sort_unstable();
        let count = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), count, "short labels collide: {labels:?}");
    }

    #[test]
    fn quality_nicks_round_trip() {
        for quality in Quality::ALL {
            assert_eq!(Quality::from_nick(quality.nick()), quality);
        }
        // Unknown nicks fall back to the free tier rather than failing.
        assert_eq!(Quality::from_nick("garbage"), Quality::Low);
    }
}
