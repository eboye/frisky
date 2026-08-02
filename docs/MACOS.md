# Porting Frisky to macOS

**Status: not planned. Frisky is a Linux application.**

This document exists so that anyone who wants to try does not have to redo the
analysis first. Nothing here is a commitment — see [Scope](#scope-and-expectations)
at the end for what the maintainer will and will not take on.

Nothing in this document has been tested on macOS. It is a reading of the
codebase plus what is known about the stack, and the uncertain parts are marked
as such. **Do the spike below before trusting any of it.**

## Start here: the two-hour spike

Almost every platform-coupled subsystem in Frisky already degrades gracefully,
because it has to on Linux too — a missing keyring, an absent MPRIS bus and an
unavailable audio device are all handled by carrying on. That means the app may
simply *run* on macOS, badly, without any porting work at all. Finding out is
cheap and answers most of the open questions at once.

```sh
brew install rust pkg-config gtk4 libadwaita glib \
    gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-libav

git clone https://github.com/eboye/frisky && cd frisky
cargo run
```

Use `cargo run` specifically. A debug build points `GSETTINGS_SCHEMA_DIR` at the
schema `build.rs` compiled, so it needs no system-wide install; a release build
does not, and will abort inside GIO on a missing schema.

What the spike tells you, in order of what it settles:

1. **Does it compile?** `oo7` is the likeliest failure — it is a Secret Service
   client and may not build on macOS at all. If so, that is the first PR (see
   below) rather than a dead end.
2. **Does it play?** If `playbin3` picks `osxaudiosink` on its own, the core
   works and the rest is polish.
3. **How bad does it look?** This settles the styling question definitively, and
   is the thing a document cannot tell you.
4. **Does it follow system dark mode?** Probably not. See below.

Expected at runtime, all non-fatal: MPRIS fails to publish and logs a warning;
the keyring lookup returns `None` and the app stays on the free tier; the output
device picker lists only "System Default".

## What ports unchanged

Around 1,500 lines have no platform coupling at all: `api/` (HTTP client, serde
models, the now-playing WebSocket), `channel.rs`, `artwork.rs` and `event.rs`.
`glib::user_cache_dir()` already resolves to `~/Library/Caches`, so even the
artwork cache lands in the right place.

GStreamer works on macOS. `playbin3`, ICY metadata via `souphttpsrc`, and the
`level` element that drives the visualiser are all available.

The GTK widget layer — `window.rs`, `widgets/` and `style.css` — is portable in
the sense that it will render. Whether it *belongs* is a separate question,
addressed under [Styling](#styling).

## What needs replacing

| Area | Problem | Replacement |
|---|---|---|
| `mpris.rs` (368 lines) | MPRIS is a D-Bus protocol; Linux only | `MPNowPlayingInfoCenter` and `MPRemoteCommandCenter` from `MediaPlayer.framework`, via `objc2`. Full rewrite, but the window already exposes every operation it needs — `activate_channel`, `step_channel`, `toggle_playback_external`, `start_playback_external`, `stop_playback_external` and `set_volume_external` — so the replacement is a new backend against an existing seam, not a redesign |
| `auth.rs` keyring | Secret Service is D-Bus | Keychain. Small — the `keyring` crate covers both, or `security-framework` directly |
| Notifications in `app.rs` | GIO has no macOS notification backend as far as is known | `UNUserNotificationCenter`. Requires a signed bundle, so it cannot be tested unsigned |
| `audio.rs` device picker | `osxaudiosink` exposes `device` as an `AudioDeviceID` integer, not a string. `element_device_id` only reads string properties, so nothing is listed | Add an integer branch. Fails safe today — the list falls back to System Default — so this is polish, not a blocker |
| `data/*.desktop`, `*.service.in` | Linux desktop integration | `Info.plist` in an app bundle |

### Two less obvious ones

**libadwaita is not supported on macOS.** It builds — Homebrew ships it — but
upstream targets GNOME and does not test elsewhere. The concrete consequence to
expect is `AdwStyleManager`: it follows the system colour scheme through GNOME
settings, so the app probably will not follow macOS Appearance. Frisky's own
gradients are identical in light and dark by design, so this affects surfaces
rather than branding.

**Single-instance behaviour depends on D-Bus.** `GApplication` enforces
uniqueness over the session bus, and macOS has none, so launching twice may give
two copies. Worth checking early, because the whole `app.rs` activation path
assumes a second launch raises the existing window.

## Styling

The honest split:

**Carries over unchanged** — the four brand gradients, cover art, the
auto-ranging visualiser, the buffering wave, the channel pills, and the mini
player's layer stack. It is all GTK CSS and Cairo drawing. `filter: blur()` and
`transform: scale()` are GTK CSS features and should hold up under the GL
renderer.

**Does not** — GTK draws its own window controls, so a GNOME-style close button
appears where the traffic lights belong. The mini player merges the title bar
into the control row, which is the nicest thing about compact mode on GNOME and
the most alien thing about it on macOS. There is no native menu bar integration.

So: it will look like FRISKY, and it will look like a GNOME app. If that is
acceptable, the GTK path is viable. If it is not, no amount of CSS fixes it, and
the second approach below is the answer.

## Two approaches

### A — Port the GTK application

Reuses most of the code. Rewrite MPRIS, swap the keyring and notifications, fix
the device picker, then solve bundling.

*For:* fastest route to something that runs; one codebase.
*Against:* looks foreign; you own a second packaging pipeline permanently; you
depend on libadwaita continuing to work on a platform it does not target.

### B — Shared Rust core, native front end

Extract `api/`, `channel.rs`, `artwork.rs` and `event.rs` into a crate — they
are already free of UI and platform assumptions — and write a SwiftUI front end
against it.

`AVPlayer` handles the streams and surfaces ICY titles through
`AVPlayerItem.timedMetadata`, so the metadata story survives. Now Playing, media
keys and Keychain all come free. Critically, **there is no GStreamer to bundle**,
which removes most of the notarization difficulty.

*For:* genuinely native; far easier to distribute.
*Against:* a second UI to maintain; the visualiser needs an
`MTAudioProcessingTap` to get at audio levels, which is fiddly.

*Recommendation:* if the goal is "I want to listen on my Mac", take A and never
notarize it. If the goal is shipping to other people, take B — and the shared
core is a good idea regardless.

## Packaging is the real cost

Not the code. In rough order of how much time each will take:

- Bundling GStreamer into a `.app` means relocating dylibs with
  `install_name_tool` and getting the plugin registry right. This is the same
  class of problem as `build-aux/appimage-apprun-hook.sh`, which took three
  separate corrections to get right on Linux.
- **The plugin scanner spawns a helper process, which conflicts with the
  hardened runtime and library validation** that notarization requires. Prototype
  this before anything else — it is the one item that could make approach A
  impractical rather than merely tedious.
- Codesigning and notarization need an Apple Developer account, $99/year.
- A universal binary means building the whole native stack for both arm64 and
  x86_64.
- Expect 150–250 MB bundled.

## A good first pull request

Independent of which approach wins, and reviewable without a Mac:

**Make the Linux-only dependencies target-conditional.** Move `oo7` and
`mpris-server` under `[target.'cfg(target_os = "linux")'.dependencies]`, put
`mod mpris` behind a `cfg`, and give `auth.rs` a small trait with a Linux
implementation and a stub. That alone may be the difference between "does not
compile" and "runs with pieces missing", and it costs the Linux build nothing.

## What "done" would look like

Rough acceptance criteria, so scope does not drift:

1. Plays all four channels with working volume
2. Now-playing metadata, cover art and the tracklist update
3. The visualiser moves
4. Media keys and the Now Playing widget work
5. Preferences persist across restarts
6. Ships as a signed, notarized `.app` or a documented Homebrew cask
7. **The Linux build is unaffected** — no regressions, CI still green

Items 1–3 are the spike plus modest work. Items 4 and 6 are the bulk of it.

## Scope and expectations

- macOS support is **not on the roadmap** and the maintainer does not use macOS.
- A pull request will be reviewed on its merits, but **cannot be tested by the
  maintainer**, so it must not risk the Linux build. Target-conditional
  dependencies and `cfg` gates are the mechanism.
- Anything that degrades the Linux experience to accommodate macOS will be
  declined.
- If the port turns out to want a different architecture — approach B — that is
  a reasonable outcome, and it may belong in a separate repository. Say so
  early rather than carrying a fork.
- Please comment on the tracking issue before starting, so effort is not
  duplicated.
