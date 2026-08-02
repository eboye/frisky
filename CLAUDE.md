# Working in this repository

Frisky is an unofficial native GNOME client for FRISKY Radio: GTK4 +
libadwaita, Rust, GStreamer for playback. Read `docs/ARCHITECTURE.md` before
changing anything structural; this file is the short version plus the traps.

## Commands

```sh
cargo run                              # build.rs compiles the schema, so this just works
cargo test --all
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

CI runs `fmt --check`, `clippy -D warnings`, `test --locked`, an `appstreamcli`
validation and a `cargo audit`. Run at least fmt, clippy and tests before
pushing — CI is not a substitute for checking your own work.

Bumping the version means editing `Cargo.toml`, `build-aux/PKGBUILD` and adding
a `<release>` to the metainfo, then `cargo update -p frisky-gtk --offline` so
`Cargo.lock` stays in sync. CI builds `--locked` and will fail otherwise.

## House style

- **Comments say why, not what.** The code already says what. Match the density
  and tone of what is there; do not narrate obvious lines.
- **Every behavioural change gets a test.** Prefer a test that would have
  caught the bug over one that restates the implementation.
- **Tests must not touch the user's real data.** Take a directory or budget as
  a parameter rather than reaching for the live cache — `artwork::prune` is
  the pattern to copy. A test that deletes someone's cover art is a bug.
- **No GTK or GDK type ever crosses a thread boundary.** They are not `Send`.
  Artwork travels as `Vec<u8>` and is decoded into a texture on the main
  thread. If you find yourself wanting to send a widget, you want an `AppEvent`.
- **Degrade, do not fail.** Missing artwork, a locked keyring, an absent audio
  device and an unparseable API field should all leave the app playing.

## Verification standards

Claims in this codebase were established by running things, not by reasoning
about them. Keep that up.

- **`flatpak-builder` can exit 0 without installing anything.** Verify the
  install by its timestamp (`flatpak info io.github.eboye.Frisky`), never by
  the exit code.
- **A local debug build will not take the D-Bus name if the Flatpak is
  running.** It silently exits as a secondary instance, so you end up probing
  the installed app and thinking your change did nothing. Use
  `dbus-run-session -- ./target/debug/frisky-gtk` for anything that touches
  actions, MPRIS or notifications.
- **`gdbus call ... org.gtk.Actions.List`** is the quickest way to prove an
  action actually exists. Two shipped menu items turned out to be dead this
  way.
- Test GStreamer behaviour against a real pipeline. `audiotestsrc` is enough.

## Traps

Each of these cost real debugging time. They are not hypothetical.

1. **GTK4 `overflow` is a widget property, not CSS.** `overflow: hidden` in a
   stylesheet does nothing; set it on the widget.
2. **Channel CSS classes leak through descendant selectors.** Putting a
   `.channel-*` class on an ancestor makes every pill match, and the last rule
   in the file wins — which is why the gradients all went black once. Pill
   rules use the direct-child combinator (`.channel-deep > .pill-surface`) on
   purpose. The mini player's gradient lives on `.compact-tint`, never on the
   window.
3. **GStreamer's `level` element reports `rms` as a `GValueArray`**, not a
   `GstValueList`. Reading it as the wrong type fails silently and shows up
   only as a visualiser that never moves. `player.rs` probes three types and
   has an integration test driving a real element — keep it.
4. **A pipeline flushes its bus on READY→NULL**, so no `StateChanged` to
   `Null` is ever delivered. Do not write code waiting for one; `stop()` sets
   the state directly.
5. **`GNotification`'s default action must name an action that exists.**
   GApplication does not register `app.activate` for you.
6. **`win.show-help-overlay` only exists if GTK finds the resource** at
   `/io/github/eboye/Frisky/gtk/help-overlay.ui`. The path is GTK's to choose,
   not ours. Moving or renaming that file silently kills the menu item.
7. **The crate feature floors (`v4_12`, `v1_5`) become pkg-config minimums.**
   Raising them raises the minimum GTK/libadwaita for every distro build. Do
   not raise them to use an API casually.
8. **Do not build the Flatpak in `/tmp`.** It is a RAM-backed tmpfs on typical
   systems; a release build there will exhaust memory and fill it. Build
   out-of-tree on real disk (see the README), because `flatpak-builder`'s
   output contains a symlink to `/run` that sends cargo into filesystem loops.
   `Cargo.toml`'s `exclude` guards the same problem.
9. **In the release workflow, `actions/checkout` must come before
   `download-artifact`.** Checkout cleans the workspace, so the other order
   deletes the artifacts and publishes an empty release while reporting
   success. There is an explicit assertion step guarding this — leave it.
10. **`linuxdeploy-plugin-gtk` sets `GTK_THEME`**, which makes GTK load its own
    stylesheet instead of libadwaita's and renders the AppImage like an old
    GTK3 app. `build-aux/appimage-apprun-hook.sh` unsets it. The same plugin
    forces X11 and points `GST_PLUGIN_SCANNER` at a path that does not exist;
    the hook fixes both.

## Project decisions

These are settled. Do not re-litigate them without being asked.

- **No stream recording.** Recording mixes to disk was considered and declined
  on terms-of-service grounds. Do not implement it or suggest it.
- **Stop, not pause.** Live radio has no seekable timeline; pausing just
  accumulates stale audio. MPRIS still advertises Pause and maps it to stop.
- **Per-track now-playing is impossible.** The ICY title is the *mix*, and the
  API's tracklist carries no per-track timing. Progress is through the mix,
  from the schedule. Do not invent a "current track".
- **No trademarked assets are bundled.** Channel identity is brand gradients
  and the wordmark, both reproduced in CSS.
- **Commit messages carry no tooling trailers.** No `Claude-Session:` line, no
  "Generated with" footer, no co-author line for a tool.

## Security notes

- The subscriber token lives in the Secret Service, never on disk. It is
  percent-encoded into the stream URL and redacted from logs — keep both.
- Artwork URLs come from the API and are required to be `https`.
- Workflow tokens default to `contents: read`; only the publish job widens it.
- The Flatpak action is pinned to a commit because it runs privileged with
  access to the checkout. Do not relax it to `@master`.
- `cargo audit` runs in CI on changes and weekly. It is deliberately *not* in
  the release workflow, so an advisory published mid-incident cannot block
  shipping the fix for it.
