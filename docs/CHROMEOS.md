# Frisky on ChromeOS

**This is not a port.** ChromeOS runs Linux applications natively through
Crostini, so Frisky should already work there — unmodified, today. What is
missing is that nobody has verified it, nothing documents the install path, and
**every release artifact is x86_64 while a large share of Chromebooks are ARM**.

That makes this a much smaller and better-defined piece of work than the
[macOS](MACOS.md) or [Windows](WINDOWS.md) analyses, both of which are real
ports. Nothing here has been tested on a Chromebook; it is a reading of what
Crostini provides.

## Background

Crostini is a Debian container inside a lightweight VM, integrated with the
ChromeOS shell: Linux apps get windows, launcher entries and file sharing.
Enabled under Settings → Advanced → Developers → Linux development environment.
Available on most Chromebooks from ChromeOS 69 onward, though managed devices
can have it disabled by policy.

Frisky needs GTK4, libadwaita, GStreamer and a network connection. Crostini
provides all of them, so there is nothing to port.

## Install paths, best first

Untested, in expected order of reliability:

**Binary tarball.** The simplest thing that should work — no sandboxing, no
FUSE, no nested containers.

```sh
sudo apt install libgtk-4-1 libadwaita-1-0 \
    gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
    gstreamer1.0-plugins-bad gstreamer1.0-libav
# then extract the release tarball and run build-aux/install.sh
```

Note that Debian stable in Crostini may ship GTK older than the 4.12 the crate
requires; check before assuming.

**Flatpak.** `sudo apt install flatpak` works in Crostini, though running a
sandbox inside a container inside a VM is where surprises live. If it works it
is the best option, since it carries its own GTK and GStreamer and sidesteps the
Debian-version question entirely.

**AppImage — expect friction.** AppImages need FUSE to mount themselves, and
Crostini does not reliably provide it. Use
`./Frisky-*.AppImage --appimage-extract-and-run`, or extract once with
`--appimage-extract` and run the result.

## What to expect

| Area | Expectation |
|---|---|
| Playback | Should work. Crostini bridges audio to CRAS and presents PulseAudio in the container, which `playbin3` handles |
| Now playing, artwork, tracklist | Should work — ordinary HTTPS |
| Visualiser | Should work, but watch performance. GPU acceleration in Crostini varies by device, and a software fallback may make it choppy |
| Notifications | Expected to surface in the ChromeOS notification centre; worth confirming |
| MPRIS | Will publish inside the container, but **ChromeOS media controls will not see it**. No media-key or lock-screen integration. Not fixable from this side |
| Keyring | **gnome-keyring is not installed by default**, so the token lookup fails and the app stays on the free tier. `sudo apt install gnome-keyring` should fix it. Degrades gracefully either way |
| Output device picker | Likely lists only "System Default", since Crostini presents a single bridged sink |

Every one of those failure modes is already handled — the app is built to carry
on without a keyring, without MPRIS and without a device list, because those can
all be absent on Linux too.

## The actual gap: ARM

**All five release artifacts are x86_64 only.** The Flatpak, AppImage, Arch
package and tarball are all built on `ubuntu-24.04` x86_64 runners, and the
PKGBUILD declares `arch=('x86_64')`.

Many Chromebooks use ARM chips — MediaTek, Rockchip, Qualcomm — and on those,
*none* of the published artifacts run. Building from source in Crostini would
work, but that is a poor answer for a music player.

**This is worth fixing regardless of ChromeOS.** aarch64 builds would also serve
Raspberry Pi, Asahi Linux on Apple silicon, and ARM Linux laptops. GitHub now
provides ARM runners, and the Flatpak and tarball jobs should port with little
more than a matrix entry. It is probably the single highest-value item in this
document, and it needs no Chromebook to implement — only to verify.

## What would close this out

1. Confirm Frisky runs under Crostini and say which install path worked
2. Note anything from the table above that behaves differently than expected
3. Add a short ChromeOS section to the README covering the install path
4. *Optionally, and valuable independently:* add aarch64 to the release matrix

Steps 1–3 need a Chromebook and about an hour. Step 4 needs neither, but cannot
be verified for ChromeOS without one.

## Scope and expectations

- The maintainer does not have a Chromebook and cannot test any of this.
- ChromeOS is not a supported platform, but since it runs Linux binaries, it
  costs nothing to document what works — and a report saying "this all worked"
  is as useful as a patch here.
- aarch64 release artifacts would be accepted on their own merits, independent of
  ChromeOS.
- Please comment on the tracking issue before starting.
