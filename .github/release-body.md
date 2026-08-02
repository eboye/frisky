<div align="center">

<img src="https://raw.githubusercontent.com/eboye/frisky/main/data/screenshots/mini.png" width="560" alt="The Frisky mini player">

</div>

A native GNOME player for [FRISKY Radio](https://frisky.fm) — four channels of
electronic music, the cover art and tracklist of the DJ mix currently on air, and
a live visualiser drawn over it.

> **Unofficial.** Not affiliated with or endorsed by FRISKY. Channel names,
> artwork and audio belong to FRISKY and its artists.

## Install

**Flatpak** — self-contained, brings its own GTK and GStreamer. The one to pick
if you are unsure.

```sh
flatpak install --user frisky.flatpak
flatpak run io.github.eboye.Frisky
```

**AppImage** — self-contained. GTK, libadwaita and GStreamer are bundled, so
nothing needs installing. Requires glibc 2.39 or newer.

```sh
chmod +x Frisky-*-x86_64.AppImage
./Frisky-*-x86_64.AppImage
```

**Arch Linux** — links against the system GTK and GStreamer, so it stays small
and follows toolkit updates from pacman. The `PKGBUILD` is attached too.

```sh
sudo pacman -U frisky-*-x86_64.pkg.tar.zst
```

**Binary tarball** — for a system-style install.

```sh
tar -xzf frisky-*-x86_64-linux.tar.gz -C ~/.local
glib-compile-schemas ~/.local/share/glib-2.0/schemas
```

## Good to know

- Streaming at **96 kbps is free**. The 128 and 320 kbps tiers need a FRISKY
  subscription; log in under Preferences and the app checks entitlement before
  switching, falling back rather than failing.
- **<kbd>Ctrl</kbd>+<kbd>M</kbd>** collapses the window into a mini player — one
  gradient row with the track, mix progress and channel chips. It also appears
  on its own if you just make the window short.
- Play, stop and switch channels from the GNOME top bar, media keys and lock
  screen via MPRIS.
- Live radio carries no per-track information, so "now playing", the cover art
  and the notifications all describe the **mix** on air, not individual tracks.
  The progress bar is real, though — it comes from the broadcast schedule.

---
