<div align="center">

<img src="data/icons/io.github.eboye.Frisky.svg" width="128" alt="Frisky icon">

# Frisky

**A native GNOME player for [FRISKY Radio](https://frisky.fm)**

[![CI](https://github.com/eboye/frisky/actions/workflows/ci.yml/badge.svg)](https://github.com/eboye/frisky/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/eboye/frisky?include_prereleases&sort=semver)](https://github.com/eboye/frisky/releases/latest)
[![License](https://img.shields.io/badge/license-GPL--3.0--or--later-blue)](COPYING)

Built with GTK4, libadwaita and GStreamer.

</div>

> [!NOTE]
> **Unofficial.** Not affiliated with or endorsed by FRISKY. Channel names,
> artwork and audio belong to FRISKY and its artists.

---

## Features

- **All four channels** — Frisky, Deep, Chill and Classics, each in its own brand gradient
- **Cover art and tracklist** for the DJ mix currently on air
- **Live visualiser** drawn over the artwork, fading away on hover
- **MPRIS** — play, stop and switch channels from the GNOME top bar, media keys and lock screen
- **Compact player** (<kbd>Ctrl</kbd>+<kbd>M</kbd>) — a single gradient bar for the corner of your screen
- **Notifications** when the mix changes
- Follows the system light/dark preference, and remembers your channel and volume

## Install

### Flatpak (recommended)

Download `frisky.flatpak` from the [latest release](https://github.com/eboye/frisky/releases/latest):

```sh
flatpak install --user frisky.flatpak
flatpak run io.github.eboye.Frisky
```

### AppImage

```sh
chmod +x Frisky-*-x86_64.AppImage
./Frisky-*-x86_64.AppImage
```

Needs GTK 4.12+, libadwaita 1.5+ and the GStreamer plugin sets from your distro.
If those are missing, use the Flatpak — it carries its own.

### Binary tarball

```sh
tar -xzf frisky-*-x86_64-linux.tar.gz -C ~/.local
glib-compile-schemas ~/.local/share/glib-2.0/schemas
```

### From source

```sh
git clone https://github.com/eboye/frisky.git
cd frisky
cargo build --release
./build-aux/install.sh              # into ~/.local
```

## Building

Any current GNOME development environment works: GTK 4.12+, libadwaita 1.5+,
GStreamer 1.20+ with the base/good/libav plugin sets, OpenSSL, and Rust 1.85+.

```sh
cargo run       # build.rs compiles the GSettings schema, so this just works
cargo test
```

<details>
<summary>Distro dependencies</summary>

**Fedora**

```sh
sudo dnf install gtk4-devel libadwaita-devel gstreamer1-devel \
    gstreamer1-plugins-base-devel openssl-devel \
    gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-libav
```

**Debian / Ubuntu 24.04+**

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev libgstreamer1.0-dev \
    libgstreamer-plugins-base1.0-dev libssl-dev \
    gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-libav
```

**Arch**

```sh
sudo pacman -S gtk4 libadwaita gst-plugins-base gst-plugins-good \
    gst-plugins-bad gst-libav openssl
```

</details>

### Flatpak

```sh
flatpak install flathub org.gnome.Platform//49 org.gnome.Sdk//49 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08

# Build outside the source tree: flatpak-builder's output contains a symlink
# to /run, and cargo walks itself into filesystem loops if it lives here.
flatpak-builder --user --install --force-clean /tmp/frisky-build \
    build-aux/io.github.eboye.Frisky.json
```

After changing dependencies, regenerate the vendored crate list the offline
Flatpak build needs:

```sh
pip install aiohttp toml tomlkit
curl -O https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
```

### Releasing

Push a tag and the [release workflow](.github/workflows/release.yml) builds the
Flatpak bundle, AppImage and binary tarball, then publishes them:

```sh
git tag -a v0.1.0 -m "v0.1.0" && git push origin v0.1.0
```

## How it talks to FRISKY

Everything comes from the same public API the web player uses. Nothing here needs
credentials except the higher bitrates.

| What | Where |
|---|---|
| Channels, current mix, tracklist | `GET https://api.frisky.fm/v3/stations` |
| Schedule pushes | `wss://api.frisky.fm/v3/stations/nowplaying` |
| Cover art | mix → `GET /v3/shows/{id}` → `album_art` |
| Audio | `https://stream.{channel}.friskyradio.com/{mount}` |

The now-playing socket sends a schedule of roughly ten upcoming mixes per
channel, about fourteen hours ahead. The app uses it to work out exactly when the
current mix ends and sleeps until then instead of polling — with a half-hour
ceiling as a staleness guard, and a fixed fallback when the socket is down.

### The streams are not bot-blocked

FRISKY does **not** block command-line players, and no User-Agent spoofing is
needed. The `401 Authentication Required` from `mp3_mid` and `mp3_high` is the
subscription paywall, not a bot filter. The bare host URL is an alias for
`mp3_low` and is served to anyone:

```sh
mpv --no-video https://stream.classics.friskyradio.com/
```

### Audio quality

| Tier | Mount | Bitrate | Subscription |
|---|---|---|---|
| Low | `mp3_low` | 96 kbps | — |
| High | `mp3_mid` | 128 kbps | required |
| HI-FI | `mp3_high` | 320 kbps | required |

Logging in exchanges your credentials for a token, kept in the system keyring and
never written to disk. Before opening a premium mount the app asks
`/v3/subscriptions/validate-streaming`; if you are not entitled it says so and
falls back to 96 kbps rather than failing.

## Two things the API cannot do

**Per-track now-playing does not exist.** On live radio the ICY title is the name
of the *mix*, and the tracklist carries no per-track timing — so nothing can say
which track is playing at minute 31 of a two-hour set. "Now playing" and the
cover art refer to the mix on air, and the tracklist is shown whole and
unhighlighted rather than guessing at a current row. Notifications fire on mix
changes, roughly hourly.

**The visualiser is not a waveform of the whole mix.** [Decibels][decibels] can
draw one because it has the file and decodes it up front. A live stream has no
file and no future to draw, so this shows real amplitude as it arrives, scrolling
right to left. It auto-ranges to the levels actually coming in: broadcast audio is
heavily limited and sits in a narrow band near the top, which a fixed dBFS scale
would draw as a flat wall.

[decibels]: https://gitlab.gnome.org/GNOME/decibels

## Layout

```
src/
├── main.rs          entry point, tokio runtime, stylesheet
├── app.rs           application object, actions, notifications
├── window.rs        the window and its event loop
├── channel.rs       the four channels, stream URLs, quality tiers
├── player.rs        GStreamer pipeline, ICY titles, audio levels
├── mpris.rs         MPRIS interface
├── artwork.rs       cover art fetching and disk cache
├── auth.rs          login and keyring storage
├── preferences.rs   quality and account settings
├── event.rs         the tokio → GTK event channel
├── api/             HTTP client, models, now-playing socket
└── widgets/         channel pills, tracklist, visualiser
```

Background work runs on a tokio runtime and reaches the GTK main loop as
`AppEvent`s over an async channel; no GTK type ever leaves the main thread.

## Licence

[GPL-3.0-or-later](COPYING).
