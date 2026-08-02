# Porting Frisky to Windows

**Status: not planned. Frisky is a Linux application.**

This document exists so that anyone who wants to try does not have to redo the
analysis first. Nothing here is a commitment — see [Scope](#scope-and-expectations)
at the end.

Nothing here has been tested on Windows. It is a reading of the codebase plus
what is known about the stack, and the uncertain parts are marked as such.
**Do the spike below before trusting any of it.**

If you are also weighing macOS, read [MACOS.md](MACOS.md) — much of the analysis
is shared, and the differences are called out here.

## Verdict up front

**Windows is the more tractable of the two ports**, for one reason that has
nothing to do with code: there is no notarization, no hardened runtime, and no
library validation. The single scariest item in the macOS analysis — GStreamer's
plugin scanner spawning a helper process that fights Apple's signing
requirements — simply does not exist here. An unsigned build runs, with a
SmartScreen warning.

The Windows equivalent of MPRIS is also better trodden from Rust than the macOS
one. `SystemMediaTransportControls` is reachable through the official `windows`
crate, and gives the Windows 10/11 media overlay and media keys.

## Start here: the two-hour spike

Almost every platform-coupled subsystem in Frisky already degrades gracefully,
because it has to on Linux too — a missing keyring, an absent MPRIS bus and an
unavailable audio device are all survivable. The app may simply *run*, badly,
with no porting work.

The path of least resistance is MSYS2, which packages the whole stack:

```
pacman -S mingw-w64-x86_64-rust mingw-w64-x86_64-pkgconf \
          mingw-w64-x86_64-gtk4 mingw-w64-x86_64-libadwaita \
          mingw-w64-x86_64-gstreamer mingw-w64-x86_64-gst-plugins-base \
          mingw-w64-x86_64-gst-plugins-good mingw-w64-x86_64-gst-plugins-bad \
          mingw-w64-x86_64-gst-libav

cargo run
```

Use `cargo run` specifically: a debug build points `GSETTINGS_SCHEMA_DIR` at the
schema `build.rs` compiled, so it needs no install. A release build does not,
and will abort inside GIO on a missing schema.

What it settles, in order:

1. **Does it compile?** `oo7` is the likeliest failure — it is a Secret Service
   client and may not build off Linux at all. If so, that is the first PR (below)
   rather than a dead end.
2. **Does it play?** If `playbin3` picks `wasapi2sink` on its own, the core works.
3. **How bad does it look?** The question a document cannot answer for you.
4. **Does one instance stay one instance?** See the caveat below.

Expected at runtime, all non-fatal: MPRIS fails to publish and logs a warning;
the keyring lookup returns `None` and the app stays on the free tier; the output
device picker lists only "System Default".

## What ports unchanged

Around 1,500 lines have no platform coupling: `api/` (HTTP client, serde models,
the now-playing WebSocket), `channel.rs`, `artwork.rs` and `event.rs`.

**TLS is already portable.** The crate uses `native-tls` rather than OpenSSL for
both `reqwest` and `tokio-tungstenite`, which means SChannel on Windows with no
extra dependency. `openssl` appears in the Arch PKGBUILD as a system library, not
as a build requirement for this code.

`glib::user_cache_dir()` resolves to `%LOCALAPPDATA%`, so the artwork cache lands
somewhere sensible without changes.

GStreamer is well supported on Windows and has official installers as an
alternative to MSYS2.

## What needs replacing

| Area | Problem | Replacement |
|---|---|---|
| `mpris.rs` (368 lines) | MPRIS is D-Bus | `SystemMediaTransportControls` via the `windows` crate. The window already exposes every operation needed — `activate_channel`, `step_channel`, `toggle_playback_external`, `start_playback_external`, `stop_playback_external`, `set_volume_external` — so this is a new backend against an existing seam |
| `auth.rs` keyring | Secret Service is D-Bus | Windows Credential Manager. Small — the `keyring` crate covers both |
| Notifications in `app.rs` | No known GIO backend on Windows | Toast notifications. Needs an AppUserModelID and a Start Menu shortcut, or MSIX packaging — toasts from an unpackaged, unregistered app are awkward |
| `audio.rs` device picker | Written around `pulsesink`'s `device` and `pipewiresink`'s `target-object`, both strings. WASAPI sinks spell it differently | Extend `element_device_id`. Fails safe today — falls back to System Default — so polish, not a blocker |
| `data/*.desktop`, `*.service.in`, gschema install | Linux desktop integration | Installer-managed; schemas compiled into the install tree with `GSETTINGS_SCHEMA_DIR` set, or rely on GIO's registry backend |

### Three uncertainties worth resolving early

**libadwaita is not supported on Windows.** MSYS2 packages it, so it builds, but
upstream targets GNOME. Expect `AdwStyleManager` not to follow the Windows
light/dark setting. Frisky's gradients are identical in both by design, so this
affects surfaces rather than branding.

**Single-instance behaviour depends on GDBus.** `GApplication` enforces
uniqueness over the session bus. GLib on Windows does provide a D-Bus
implementation, but whether the uniqueness path works cleanly here is exactly
the sort of thing to check in the spike rather than assume — the whole `app.rs`
activation path assumes a second launch raises the existing window.

**Client-side decorations.** GTK4 draws its own title bar and window controls.
Windows users are more accustomed to native ones than macOS users are to
non-native traffic lights, so this is less jarring than the equivalent macOS
problem, but it is still visibly not a Windows application.

## Styling

Same split as macOS. **Carries over unchanged:** the four brand gradients, cover
art, the auto-ranging visualiser, the buffering wave, channel pills, and the mini
player's layer stack — all GTK CSS and Cairo drawing. **Does not:** window
controls, title bar conventions, and system accent/theme following.

It will look like FRISKY, and it will look like a GTK application. Whether that
is acceptable is a judgement call, not a technical one.

## Packaging

Easier than macOS, and the main reason to prefer this port if you are choosing.

- Bundle GTK, libadwaita and GStreamer DLLs alongside the binary. GStreamer needs
  its plugin directory located correctly — the same class of problem as
  `build-aux/appimage-apprun-hook.sh`, but without a hardened runtime fighting it.
- An installer via WiX (MSI) or NSIS. `cargo-wix` and `cargo-packager` both exist.
- **Code signing is optional.** Unsigned binaries run; SmartScreen shows a warning
  that fades once the binary earns reputation. An OV certificate is roughly
  $200–400/year if that matters to you.
- Expect 100–200 MB bundled.
- GitHub Actions provides Windows runners, so CI is not an obstacle.

## A good first pull request

Reviewable without a Windows machine, and shared with the macOS effort:

**Make the Linux-only dependencies target-conditional.** Move `oo7` and
`mpris-server` under `[target.'cfg(target_os = "linux")'.dependencies]`, put
`mod mpris` behind a `cfg`, and give token storage a small trait with a Linux
implementation and a stub. That may be the difference between "does not compile"
and "runs with pieces missing", and it costs the Linux build nothing.

## What "done" would look like

1. Plays all four channels with working volume
2. Now-playing metadata, cover art and the tracklist update
3. The visualiser moves
4. Media keys and the SMTC overlay work
5. Preferences persist across restarts
6. Ships as an installer or a documented `winget` manifest
7. **The Linux build is unaffected** — no regressions, CI still green

Items 1–3 are the spike plus modest work. Items 4 and 6 are the bulk.

## Scope and expectations

- Windows support is **not on the roadmap** and the maintainer does not use
  Windows.
- A pull request will be reviewed on its merits but **cannot be tested by the
  maintainer**, so it must not put the Linux build at risk. Target-conditional
  dependencies and `cfg` gates are the mechanism.
- Anything that degrades the Linux experience to accommodate Windows will be
  declined.
- Please comment on the tracking issue before starting, so effort is not
  duplicated.
