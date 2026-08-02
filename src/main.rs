//! Frisky — an unofficial GNOME client for FRISKY Radio.

mod api;
mod app;
mod artwork;
mod audio;
mod auth;
mod channel;
mod event;
mod mpris;
mod player;
mod preferences;
mod window;

mod widgets {
    pub mod buffering;
    pub mod channel_pill;
    pub mod tracklist;
    pub mod visualizer;
}

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use std::sync::OnceLock;
use tokio::runtime::Runtime;

/// The one tokio runtime. Every socket and HTTP request lives here; the GTK
/// main loop never blocks on it.
pub fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to start the async runtime"))
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "frisky_gtk=info".into()),
        )
        .init();

    // In a debug build the schema has not been installed system-wide, so point
    // GSettings at the copy build.rs compiled. Without this, `cargo run` would
    // abort inside gio.
    #[cfg(debug_assertions)]
    if std::env::var_os("GSETTINGS_SCHEMA_DIR").is_none() {
        std::env::set_var("GSETTINGS_SCHEMA_DIR", env!("FRISKY_SCHEMA_DIR"));
    }

    gio::resources_register_include!("frisky.gresource")
        .expect("failed to register embedded resources");

    app::FriskyApplication::new().run()
}

pub(crate) fn load_stylesheet() {
    let provider = gtk::CssProvider::new();
    provider.load_from_resource("/io/github/eboye/Frisky/style.css");

    // Display is available before the app runs because GTK is initialised by
    // the Adw::Application's own startup; guard anyway rather than panic.
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
