# Contributing

Thanks for taking an interest. This covers getting set up, the dev loop, and
how a change gets from your machine to a release.

Two companion documents are worth reading first:

- **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)** — how the pieces fit together
  and why. Read this before any structural change.
- **[CLAUDE.md](CLAUDE.md)** — conventions, verification standards, settled
  decisions, and the traps in this stack that have already cost someone an
  afternoon. Short, and it will save you time.

## Setting up

You need GTK 4.12+, libadwaita 1.5+, GStreamer 1.20+ with the base/good/libav
plugin sets, OpenSSL and Rust 1.85+. The [README](README.md#building) lists the
package names for Fedora, Debian/Ubuntu and Arch.

```sh
cargo run     # build.rs compiles the GSettings schema, so this needs no install
```

## The dev loop

```sh
cargo run
cargo test --all
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Test against the native binary rather than rebuilding the Flatpak — a Flatpak
build takes minutes and the dev loop takes seconds.

**One trap worth knowing up front.** If the Flatpak is installed and running, it
owns the `io.github.eboye.Frisky` D-Bus name, so your freshly built binary will
exit immediately as a secondary instance. It looks exactly like your change
having no effect. For anything touching actions, MPRIS or notifications, use a
private bus:

```sh
dbus-run-session -- ./target/debug/frisky-gtk
```

`gdbus call --session --dest io.github.eboye.Frisky --object-path \
/io/github/eboye/Frisky --method org.gtk.Actions.List` is the fastest way to
prove an action actually exists. Two shipped menu items turned out to be dead
that way.

## What a good change looks like

- **Comments explain why, not what.** Match the density and tone of the
  surrounding code.
- **Behavioural changes come with a test.** Prefer one that would have caught
  the bug over one that restates the implementation.
- **Tests must not touch the user's real data.** Take a directory or a budget as
  a parameter rather than reaching for the live cache — `artwork::prune` is the
  pattern to copy.
- **Degrade rather than fail.** Missing artwork, a locked keyring, an absent
  audio device and an unparseable API field should all leave the app playing.
- **No GTK or GDK type crosses a thread boundary.** They are not `Send`.

Commit messages: a short imperative subject, then prose explaining why the
change is needed. No tooling trailers — no `Claude-Session:` line, no "Generated
with" footer, no co-author line for a tool.

## Before opening a pull request

CI runs `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --locked`,
an AppStream validation and a `cargo audit`. Run the first three locally; they
take seconds and CI is not a substitute for checking your own work.

If you touched `data/`, also run:

```sh
appstreamcli validate --no-net data/io.github.eboye.Frisky.metainfo.xml
desktop-file-validate data/io.github.eboye.Frisky.desktop
```

## Building the Flatpak

```sh
flatpak install flathub org.gnome.Platform//49 org.gnome.Sdk//49 \
    org.freedesktop.Sdk.Extension.rust-stable//25.08

# Build outside the source tree: flatpak-builder's output contains a symlink to
# /run, and cargo walks itself into filesystem loops if it lives here. The state
# dir has to move with it — flatpak-builder requires both on one filesystem.
#
# Not /tmp: on many systems that is a RAM-backed tmpfs, and a cargo release
# build there will exhaust memory and fill it. Use real disk.
flatpak-builder --user --install --force-clean \
    --state-dir=~/.cache/frisky-flatpak/state ~/.cache/frisky-flatpak/build \
    build-aux/io.github.eboye.Frisky.json
```

`flatpak-builder` can exit 0 without having installed anything, so confirm the
result by its timestamp rather than the exit code:

```sh
flatpak info io.github.eboye.Frisky | grep -iE 'version|date'
```

After changing dependencies, regenerate the vendored crate list the offline
Flatpak build needs:

```sh
pip install aiohttp toml tomlkit
curl -O https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
python3 flatpak-cargo-generator.py Cargo.lock -o build-aux/cargo-sources.json
```

## Releasing

A release is a version bump in four places, then a tag.

1. `Cargo.toml` — `version`
2. `build-aux/PKGBUILD` — `pkgver`
3. `data/io.github.eboye.Frisky.metainfo.xml` — a new `<release>` entry, written
   for users rather than as a changelog of commits
4. `cargo update -p frisky-gtk --offline` so `Cargo.lock` follows; CI builds
   `--locked` and fails otherwise

Then verify, commit, and tag:

```sh
cargo fmt --all --check && cargo clippy --all-targets --locked -- -D warnings
cargo test --all --locked
appstreamcli validate --no-net data/io.github.eboye.Frisky.metainfo.xml

git commit -am "Release v0.1.6"
git push origin main
git tag -a v0.1.6 -m "Frisky 0.1.6" && git push origin v0.1.6
```

The [release workflow](.github/workflows/release.yml) then builds and publishes
five assets: the Flatpak bundle, an AppImage, an Arch package and its PKGBUILD,
and a binary tarball.

**Check the assets, not the job status.** A release once published green with no
files attached, because `actions/checkout` ran after `download-artifact` and
cleaned the workspace. There is an assertion step guarding that now — leave it
in place.

```sh
gh release view v0.1.6 --json assets --jq '.assets[].name'
```

## Reporting bugs

Include your distro, how you installed Frisky (Flatpak, AppImage, Arch package,
tarball or source), and the output of `flatpak info io.github.eboye.Frisky` or
`frisky-gtk --version` as appropriate. For playback problems,
`RUST_LOG=frisky_gtk=debug frisky-gtk` prints what the pipeline and the
now-playing socket are doing; tokens are redacted from that output.
