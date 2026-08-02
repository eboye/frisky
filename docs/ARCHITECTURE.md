# Architecture

How Frisky is put together and why. The README covers installing and building;
`CLAUDE.md` covers conventions and the traps. This is the structural picture.

## The shape of the problem

Frisky plays four live internet radio streams and shows what is on air. That
splits cleanly into three concerns that barely touch:

- **Playback** — a GStreamer pipeline, driven by the user, reporting state and
  audio levels back.
- **Metadata** — what is airing on each channel, fetched over HTTP and pushed
  over a WebSocket, entirely independent of whether anything is playing.
- **Presentation** — a GTK window, plus MPRIS so the desktop can drive the same
  actions the window does.

The interesting design constraint is that GTK owns the main thread and its types
are not `Send`, while all the networking wants to be async. Everything below
follows from keeping those two worlds apart.

## Threading model

There is exactly one tokio runtime, in a `OnceLock` in `main.rs`. Every socket
and HTTP request lives on it. GTK never blocks on it.

The two sides communicate in one direction only, over an unbounded
`async_channel` carrying `AppEvent` (`event.rs`):

```
   tokio runtime                          GTK main thread
   ─────────────                          ───────────────
   nowplaying coordinator ─┐
   artwork downloads      ─┼─ AppEvent ──▶ window.handle_event()
   GStreamer bus watch    ─┘                    │
                                                ├─▶ widgets
                                                ├─▶ MPRIS notify
                                                └─▶ notifications
```

`AppEvent` deliberately contains no GTK type. Artwork crosses as `Vec<u8>` and
is decoded into a `gdk::Texture` on the main thread in `on_artwork`. This is the
single most important invariant in the codebase; breaking it produces crashes
that look random.

Work travels the other way by direct call, not by channel: the window holds an
`Rc<Player>` and a `RefreshHandle`, and calls them. Those are cheap and
synchronous. Only *results* need the channel.

The event loop itself is a `glib::spawn_future_local` holding a weak reference
to the window, so it ends when the window does.

## Module responsibilities

| Module | Owns |
|---|---|
| `main.rs` | tokio runtime, resource registration, stylesheet |
| `app.rs` | `AdwApplication` subclass, app actions, accelerators, notifications, shutdown |
| `window.rs` | all widget state, the event loop, playback decisions |
| `player.rs` | the GStreamer pipeline and its state machine |
| `channel.rs` | the four channels: ids, stream URLs, quality tiers, CSS classes |
| `audio.rs` | output device enumeration and sink construction |
| `event.rs` | the channel type, `AppEvent`, `NowPlaying`, mix progress |
| `api/` | HTTP client (`mod.rs`), serde models (`model.rs`), the now-playing coordinator (`nowplaying.rs`) |
| `artwork.rs` | cover art fetch, disk cache, pruning |
| `auth.rs` | login, keyring, entitlement checks |
| `mpris.rs` | the MPRIS interface |
| `preferences.rs` | the preferences dialog |
| `widgets/` | channel pills, tracklist, visualiser, buffering wave |

`window.rs` is the largest file and the one to be most careful in: it is the
only place where playback, metadata and presentation meet.

## Playback

`player.rs` wraps `playbin3` (falling back to `playbin`). A `level` element is
attached as `audio-filter` to drive the visualiser; if it is unavailable,
playback continues without one.

The state machine is deliberately small:

```
Stopped ──play()──▶ Buffering ──▶ Playing
   ▲                    │             │
   └────stop()──────────┴─────────────┘
             or retries exhausted
```

There is no Pause. For a live stream, pausing accumulates audio that then plays
back late, so the transport is play/stop and stopping tears the pipeline down to
`Null`. MPRIS still advertises `CanPause` and maps Pause to stop, because
desktops expect a pause control and media keys emit `PlayPause`.

Errors and unexpected EOS both route to `retry_or_fail`, which reconnects up to
four times two seconds apart before surfacing a toast — mirroring the web
player, which also gives up after four attempts.

**A subtlety worth knowing:** a pipeline flushes its bus on the way down to
`Null`, so the `StateChanged` message for that transition is never delivered.
The retry guard works because `stop()` sets the state directly, not because it
observes the transition.

### Volume

`playbin`'s `volume` is a linear gain, which sounds top-heavy on a slider, so
the app stores a perceptual 0.0–1.0 value and cubes it on the way in.

## Metadata

`api/nowplaying.rs` runs one coordinator task for the lifetime of the app. It
is *not* a polling loop. The WebSocket at `wss://api.frisky.fm/v3/stations/nowplaying`
pushes a schedule of upcoming entries — roughly fourteen hours ahead — so the
coordinator works out when the current mix ends and sleeps until that moment:

```
        ┌──────────────────────────────────────┐
        │  socket_loop ──schedule──▶           │
        │                          coordinator │──▶ AppEvent::NowPlaying
        │  UI refresh request ────▶            │
        │  boundary timer ────────▶            │
        └──────────────────────────────────────┘
```

Sleep is clamped to `[15s, 30min]`: never hammer the API around a boundary,
never sleep so long that a schedule revision goes unnoticed. Without a usable
schedule it falls back to a flat five minutes.

Refreshes are debounced to one per fifteen seconds regardless of trigger, and a
burst of queued requests is collapsed into a single call. Triggers include the
stream's own ICY title change, which is the fastest signal that a mix changed.

The socket reconnects with exponential backoff (2s → 120s). Two details matter:
a connection that ends within a minute does *not* reset the backoff, so a server
that accepts and immediately hangs up cannot hold the app in a tight reconnect
loop; and reads have a fifteen-minute idle timeout, because a half-open
connection left by a suspend is otherwise indistinguishable from normal silence
and never resolves itself.

### Parsing policy

`GET /v3/stations` returns all four channels in one object, so serde's
all-or-nothing parsing means one upstream field change could blank the entire
app. Station mixes are therefore parsed leniently — a malformed mix degrades to
`None` and costs only that channel — and any field the app does not actually
read is `#[serde(default)]` so its absence can never fail a parse.

## Artwork

A mix links to a show; the show carries 1200×1200 square art. Shows repeat
constantly across the schedule, so results are cached on disk at
`$XDG_CACHE_HOME/frisky-gtk/artwork/{show_id}`, extension-less because the
decoder sniffs the format.

Writes are atomic (write then rename) so a crash mid-write cannot leave a
truncated image being served as a cache hit forever. The cache is capped at
64 MiB and pruned oldest-first after each download. URLs must be `https`.

## Presentation

The window has two layouts in a `GtkStack`, switched by an `AdwBreakpoint` on
height. There is no separate "mode" state — the breakpoint keys off the window
size, so dragging the window and using the menu item cannot disagree.

The compact player is a stack of layers, bottom to top: blurred cover art across
the whole window, the channel gradient at 75%, the visualiser faint above that,
then the controls. GTK CSS does support `filter: blur()` and `transform:
scale()`, which is what makes this possible.

Both layouts get their own `Visualizer`. The one over the cover art fades out on
hover so the artwork is never permanently hidden; the compact one is a backdrop
and has no fade.

### The visualiser

Broadcast audio is heavily limited: RMS sits in a narrow band near the top, so a
fixed dBFS scale draws a nearly flat wall. `AutoRange` follows the observed
minimum and maximum — fast attack, slow release — and rescales into that band,
the way an auto-ranging meter does. A minimum span stops a steady tone being
stretched into noise, and levels below a silence threshold are left unstretched
so the noise floor between tracks is not amplified into a light show.

## MPRIS

`mpris.rs` publishes `org.mpris.MediaPlayer2.friskygtk`. Next and Previous move
between channels, which is the closest analogue radio has to tracks. Position is
always zero and `CanSeek` is false.

Stop and Pause both route through the window rather than calling the player
directly, so that a stop from the desktop cancels any in-flight premium
entitlement check exactly as the in-app button does.

## The premium path

Higher bitrates need a subscription. The flow mirrors the web player: check
`validate-streaming`, then append the token to the stream URL.

Because that check is asynchronous, every play request carries a generation
number. A stop, or a newer request, bumps the generation and the stale result is
discarded — otherwise a stream could start playing moments after the user
pressed stop. Anything unexpected answers "not entitled" and falls back to the
free 96 kbps stream with a toast, rather than producing a 401 the user cannot
interpret.

## State and settings

GSettings holds the last channel, volume, quality, notification preference and
audio device. The window follows the `audio-device` key while open, so a change
in preferences restarts the stream immediately rather than at the next channel
switch.

Restoring state deliberately does *not* start playback. An app that begins
making noise on launch is hostile.

## Packaging

Four targets, all built by `.github/workflows/release.yml`:

| Target | Notes |
|---|---|
| Flatpak | `org.gnome.Platform//49`; offline cargo build from `cargo-sources.json` |
| AppImage | linuxdeploy + gtk and gstreamer plugins, with an AppRun hook correcting three of their assumptions |
| Arch package | `build-aux/PKGBUILD`, built against system libraries |
| Tarball | plain binary plus `build-aux/install.sh` |

The crate feature floors (`v4_12`, `v1_5`) exist so the same source builds
against both the Flatpak runtime and distro packages. They become pkg-config
minimums, so raising them raises the minimum for everyone.
