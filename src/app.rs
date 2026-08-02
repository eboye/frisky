//! Application object: wiring, actions, and notifications.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gio, glib};
use std::cell::RefCell;
use tracing::warn;

use crate::api::{nowplaying, FriskyClient};
use crate::event::{self, NowPlaying};
use crate::player::Player;
use crate::window::FriskyWindow;

pub const APP_ID: &str = "io.github.eboye.Frisky";
const NOTIFICATION_ID: &str = "now-playing";

thread_local! {
    /// Lets the window hand work to the tokio side without threading the
    /// sender through every call. Main-thread only, so a thread_local is both
    /// sufficient and honest about where it may be touched.
    static EVENT_SENDER: RefCell<Option<event::Sender>> = const { RefCell::new(None) };
}

pub fn event_sender() -> Option<event::Sender> {
    EVENT_SENDER.with(|sender| sender.borrow().clone())
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FriskyApplication {
        pub window: RefCell<Option<FriskyWindow>>,
        /// Held so the pipeline can be torn down on the way out. The MPRIS
        /// server keeps its own reference for the process's lifetime, so
        /// `Drop` alone would never run.
        pub player: RefCell<Option<std::rc::Rc<Player>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FriskyApplication {
        const NAME: &'static str = "FriskyApplication";
        type Type = super::FriskyApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for FriskyApplication {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_actions();
            self.obj().setup_options();
        }
    }

    impl ApplicationImpl for FriskyApplication {
        fn startup(&self) {
            self.parent_startup();
            // GTK is initialised by now, so a display exists to attach the
            // stylesheet to.
            crate::load_stylesheet();
        }

        fn activate(&self) {
            let app = self.obj();

            // Second launch just raises the existing window.
            if let Some(window) = self.window.borrow().as_ref() {
                window.present();
                return;
            }

            let window = FriskyWindow::new(app.upcast_ref());
            match app.build_backend(&window) {
                Ok(()) => {}
                Err(error) => {
                    warn!("failed to start backend: {error:#}");
                    window.toast("Could not start audio playback.");
                }
            }

            *self.window.borrow_mut() = Some(window.clone());
            window.present();
        }

        fn shutdown(&self) {
            // Leaves the audio device released rather than relying on process
            // teardown to do it.
            if let Some(player) = self.player.borrow().as_ref() {
                player.stop();
            }
            self.parent_shutdown();
        }
    }

    impl GtkApplicationImpl for FriskyApplication {}
    impl AdwApplicationImpl for FriskyApplication {}
}

glib::wrapper! {
    pub struct FriskyApplication(ObjectSubclass<imp::FriskyApplication>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Default for FriskyApplication {
    fn default() -> Self {
        Self::new()
    }
}

impl FriskyApplication {
    pub fn new() -> Self {
        glib::Object::builder()
            .property("application-id", APP_ID)
            .property("flags", gio::ApplicationFlags::default())
            .build()
    }

    /// Starts the network and audio machinery and hands it to the window.
    fn build_backend(&self, window: &FriskyWindow) -> anyhow::Result<()> {
        let (sender, receiver) = event::channel();
        EVENT_SENDER.with(|slot| *slot.borrow_mut() = Some(sender.clone()));

        let client = FriskyClient::new()?;
        let player = Player::new(sender.clone())?;
        *self.imp().player.borrow_mut() = Some(player.clone());
        let settings = gio::Settings::new(APP_ID);

        let refresh = {
            let client = client.clone();
            let sender = sender.clone();
            crate::runtime().block_on(async move { nowplaying::run(client, sender).await })
        };

        window.attach(client.clone(), player.clone(), refresh, receiver, settings);

        // Load any stored subscriber token in the background; until it arrives
        // the app simply stays on the free tier.
        let window_ref = window.downgrade();
        glib::spawn_future_local(async move {
            let token = crate::runtime()
                .spawn(async { crate::auth::stored_token().await })
                .await
                .ok()
                .flatten();
            if let Some(window) = window_ref.upgrade() {
                window.set_token(token);
            }
        });

        crate::mpris::attach(self, window, player);
        Ok(())
    }

    /// Adds `--version`, which a bug report can quote without the reporter
    /// having to know whether they installed a Flatpak, an AppImage or a
    /// distro package.
    fn setup_options(&self) {
        self.add_main_option(
            "version",
            glib::Char::from(b'v'),
            glib::OptionFlags::NONE,
            glib::OptionArg::None,
            "Show the version and exit",
            None,
        );

        self.connect_handle_local_options(|_, options| {
            if options.contains("version") {
                println!("frisky-gtk {}", env!("CARGO_PKG_VERSION"));
                return std::ops::ControlFlow::Break(glib::ExitCode::SUCCESS);
            }
            std::ops::ControlFlow::Continue(())
        });
    }

    fn setup_actions(&self) {
        let quit = gio::ActionEntry::builder("quit")
            .activate(|app: &Self, _, _| app.quit())
            .build();

        let about = gio::ActionEntry::builder("about")
            .activate(|app: &Self, _, _| app.present_about())
            .build();

        // Named by the notification's default action. GApplication does not
        // register this itself, so without it clicking a mix notification
        // resolves to nothing at all.
        let activate = gio::ActionEntry::builder("activate")
            .activate(|app: &Self, _, _| {
                if let Some(window) = app.window() {
                    window.present();
                }
            })
            .build();

        self.add_action_entries([quit, about, activate]);

        self.set_accels_for_action("app.quit", &["<primary>q"]);
        self.set_accels_for_action("win.toggle-playback", &["space", "<primary>p"]);
        self.set_accels_for_action("win.refresh", &["<primary>r", "F5"]);
        self.set_accels_for_action("win.preferences", &["<primary>comma"]);
        self.set_accels_for_action("win.compact", &["<primary>m"]);
        self.set_accels_for_action("win.next-channel", &["<primary>n"]);
        self.set_accels_for_action("win.show-help-overlay", &["<primary>question"]);
    }

    pub fn window(&self) -> Option<FriskyWindow> {
        self.imp().window.borrow().clone()
    }

    fn present_about(&self) {
        let dialog = adw::AboutDialog::builder()
            .application_name("Frisky")
            .application_icon(APP_ID)
            .version(env!("CARGO_PKG_VERSION"))
            .developer_name("eboye")
            .license_type(gtk::License::Gpl30)
            .website("https://frisky.fm")
            .comments(
                "An unofficial GNOME client for FRISKY Radio.\n\n\
                 Not affiliated with or endorsed by FRISKY. Channel names, \
                 artwork and audio belong to FRISKY and its artists.",
            )
            .build();

        if let Some(window) = self.window() {
            adw::prelude::AdwDialogExt::present(&dialog, Some(&window));
        }
    }
}

/// Posts a desktop notification for a newly-started mix.
///
/// Fires on mix changes — roughly hourly — not per track: live radio carries no
/// per-track position, so a track-level notification is not possible.
pub fn send_notification(entry: &NowPlaying) {
    let Some(app) = gio::Application::default() else {
        return;
    };

    let notification = gio::Notification::new(&entry.display_title());
    notification.set_body(Some(&format!(
        "{} · {}",
        entry.subtitle(),
        entry.channel.title()
    )));
    notification.set_priority(gio::NotificationPriority::Low);
    notification.set_default_action("app.activate");

    app.send_notification(Some(NOTIFICATION_ID), &notification);
}
