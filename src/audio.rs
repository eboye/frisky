//! Enumerating audio output devices, so playback can be sent somewhere other
//! than the system default.
//!
//! Worth having because the system default is not always where the speakers
//! are — a machine whose default sink is an unplugged S/PDIF port plays to
//! silence, and nothing in the app looks wrong while it does.

use gst::prelude::*;
use gstreamer as gst;
use tracing::debug;

/// An audio sink the app can play to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputDevice {
    /// Stable identifier, persisted in settings. Empty means the system
    /// default.
    pub id: String,
    /// What to show in the preferences list.
    pub name: String,
}

impl OutputDevice {
    /// The entry meaning "whatever the desktop is currently using".
    pub fn system_default() -> Self {
        Self {
            id: String::new(),
            name: "System Default".to_owned(),
        }
    }
}

/// Snapshot of the available output devices, always starting with the system
/// default.
///
/// Taken fresh each time the preferences open rather than cached: devices come
/// and go as things are plugged in.
pub fn output_devices() -> Vec<OutputDevice> {
    let mut devices = vec![OutputDevice::system_default()];

    if gst::init().is_err() {
        return devices;
    }

    let monitor = gst::DeviceMonitor::new();
    // Only sinks; sources would list microphones too.
    monitor.add_filter(Some("Audio/Sink"), None);

    if monitor.start().is_err() {
        debug!("could not start the device monitor; offering the default only");
        return devices;
    }

    for device in monitor.devices() {
        if let Some(output) = describe(&device) {
            if !devices.iter().any(|existing| existing.id == output.id) {
                devices.push(output);
            }
        }
    }
    monitor.stop();

    devices
}

/// Identifier a sink element uses to name its device.
///
/// Different sinks spell this differently — pulsesink has `device`,
/// pipewiresink has `target-object` — and reading a property an element does
/// not have panics, so check before asking.
fn element_device_id(element: &gst::Element) -> Option<String> {
    for property in ["device", "target-object", "device-name"] {
        if element.find_property(property).is_some() {
            if let Some(id) = element.property::<Option<String>>(property) {
                if !id.is_empty() {
                    return Some(id);
                }
            }
        }
    }
    None
}

/// Turns a GStreamer device into something persistable.
///
/// Devices whose element exposes no usable identifier are skipped rather than
/// offered and then silently ignored.
fn describe(device: &gst::Device) -> Option<OutputDevice> {
    let element = device.create_element(None).ok()?;
    Some(OutputDevice {
        id: element_device_id(&element)?,
        name: device.display_name().to_string(),
    })
}

/// Builds the sink for `device_id`, or `None` to let playbin choose.
///
/// The element is created from the matching device rather than from a
/// hardcoded factory, so pulsesink, pipewiresink and friends all work without
/// the app knowing which one is in play.
pub fn make_sink(device_id: &str) -> Option<gst::Element> {
    if device_id.is_empty() {
        return None;
    }

    if gst::init().is_err() {
        return None;
    }

    let monitor = gst::DeviceMonitor::new();
    monitor.add_filter(Some("Audio/Sink"), None);
    if monitor.start().is_err() {
        return None;
    }

    let sink = monitor.devices().into_iter().find_map(|device| {
        let element = device.create_element(None).ok()?;
        (element_device_id(&element)? == device_id).then_some(element)
    });
    monitor.stop();

    if sink.is_none() {
        // Unplugged since it was chosen; the default beats refusing to play.
        debug!("audio device {device_id} is not present; using the default");
    }
    sink
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_entry_is_identified_by_an_empty_id() {
        // An empty id is what tells make_sink to leave playbin alone.
        assert!(OutputDevice::system_default().id.is_empty());
    }

    #[test]
    fn an_empty_id_asks_for_no_explicit_sink() {
        // playbin picks for itself, which is what "System Default" means.
        assert!(make_sink("").is_none());
    }

    #[test]
    fn the_listing_always_offers_the_system_default_first() {
        let devices = output_devices();
        assert!(
            devices.first().is_some_and(|d| d.id.is_empty()),
            "system default must lead the list"
        );
    }

    #[test]
    fn listed_devices_are_unique_and_selectable() {
        let devices = output_devices();
        for device in devices.iter().skip(1) {
            assert!(!device.id.is_empty(), "offered a device with no id");
            assert!(!device.name.is_empty(), "offered a device with no name");
        }

        let mut ids: Vec<&str> = devices.iter().map(|d| d.id.as_str()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "the same device was listed twice");
    }
}
