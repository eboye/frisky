#! /bin/bash
# Sourced by the AppImage's AppRun after the linuxdeploy plugins, correcting
# assumptions they make that do not hold for a GTK4 / libadwaita application.
#
# Named to sort last so it runs after linuxdeploy-plugin-gtk and
# linuxdeploy-plugin-gstreamer, whose exports it overrides.

# linuxdeploy-plugin-gtk pins GTK_THEME to "Adwaita:light" or "Adwaita:dark".
# For a libadwaita app that is actively harmful: setting GTK_THEME makes GTK
# load its own stylesheet instead of libadwaita's, so the app renders like an
# older GTK application. libadwaita follows the desktop colour scheme on its
# own, via the settings portal.
unset GTK_THEME

# The same plugin forces X11 because GTK3 had Wayland problems. GTK4 does not,
# and forcing XWayland costs fractional scaling and native decorations.
unset GDK_BACKEND

# linuxdeploy-plugin-gstreamer writes Debian's multiarch layout
# (usr/lib/gstreamer1.0/gstreamer-1.0/), but the scanner is bundled at
# usr/lib/gstreamer-1.0/. Pointing at a path that does not exist makes GStreamer
# fail to load every plugin that needs the out-of-process scanner.
for candidate in \
    "${APPDIR}/usr/lib/gstreamer-1.0/gst-plugin-scanner" \
    "${APPDIR}/usr/lib/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner" \
    "${APPDIR}/usr/lib/x86_64-linux-gnu/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner"
do
    if [ -x "$candidate" ]; then
        export GST_PLUGIN_SCANNER="$candidate"
        export GST_PLUGIN_SCANNER_1_0="$candidate"
        break
    fi
done

for candidate in \
    "${APPDIR}/usr/lib/gstreamer-1.0/gst-ptp-helper" \
    "${APPDIR}/usr/lib/gstreamer1.0/gstreamer-1.0/gst-ptp-helper" \
    "${APPDIR}/usr/lib/x86_64-linux-gnu/gstreamer1.0/gstreamer-1.0/gst-ptp-helper"
do
    if [ -x "$candidate" ]; then
        export GST_PTP_HELPER_1_0="$candidate"
        break
    fi
done
