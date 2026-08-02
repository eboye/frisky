//! The application window: cover art, now playing, transport, channel pills
//! and the tracklist.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use std::collections::HashMap;
use std::rc::Rc;
use tracing::{debug, warn};

use crate::api::nowplaying::RefreshHandle;
use crate::api::FriskyClient;
use crate::artwork::ArtworkCache;
use crate::channel::{Channel, Quality};
use crate::event::{AppEvent, NowPlaying, PlayerState, Receiver};
use crate::player::Player;
use crate::widgets::channel_pill::ChannelPill;
use crate::widgets::tracklist::Tracklist;
use crate::widgets::visualizer::{Visualizer, VisualizerSize};

mod imp {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[derive(gtk::CompositeTemplate, Default)]
    #[template(resource = "/io/github/eboye/Frisky/window.ui")]
    pub struct FriskyWindow {
        #[template_child]
        pub toast_overlay: TemplateChild<adw::ToastOverlay>,
        #[template_child]
        pub window_title: TemplateChild<adw::WindowTitle>,
        #[template_child]
        pub artwork_overlay: TemplateChild<gtk::Overlay>,
        #[template_child]
        pub artwork: TemplateChild<gtk::Picture>,
        #[template_child]
        pub track_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub track_subtitle: TemplateChild<gtk::Label>,
        #[template_child]
        pub play_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub play_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub play_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub play_spinner: TemplateChild<gtk::Spinner>,
        #[template_child]
        pub volume_button: TemplateChild<gtk::ScaleButton>,
        #[template_child]
        pub channel_box: TemplateChild<gtk::Box>,
        #[template_child]
        pub tracklist_group: TemplateChild<adw::PreferencesGroup>,
        #[template_child]
        pub tracklist_row: TemplateChild<adw::ExpanderRow>,
        #[template_child]
        pub view_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub toolbar_view: TemplateChild<adw::ToolbarView>,
        #[template_child]
        pub compact_bar: TemplateChild<gtk::Box>,
        #[template_child]
        pub compact_backdrop_art: TemplateChild<gtk::Picture>,
        #[template_child]
        pub compact_tint: TemplateChild<gtk::Box>,
        #[template_child]
        pub compact_visualizer_slot: TemplateChild<gtk::Box>,
        #[template_child]
        pub compact_artwork: TemplateChild<gtk::Picture>,
        #[template_child]
        pub compact_title: TemplateChild<gtk::Label>,
        #[template_child]
        pub compact_chips: TemplateChild<gtk::Box>,
        #[template_child]
        pub compact_progress: TemplateChild<gtk::ProgressBar>,
        #[template_child]
        pub compact_time: TemplateChild<gtk::Label>,
        #[template_child]
        pub compact_play_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub compact_play_button: TemplateChild<gtk::Button>,

        pub player: RefCell<Option<Rc<Player>>>,
        pub client: RefCell<Option<FriskyClient>>,
        pub refresh: RefCell<Option<RefreshHandle>>,
        pub artwork_cache: ArtworkCache,
        pub settings: RefCell<Option<gio::Settings>>,

        pub pills: RefCell<Vec<ChannelPill>>,
        pub now_playing: RefCell<HashMap<Channel, NowPlaying>>,
        pub selected: Cell<Channel>,
        /// Show whose artwork is on screen, so repeat events are no-ops.
        pub displayed_show: Cell<Option<u64>>,
        /// Suppresses the settings write while loading the stored volume.
        pub loading_volume: Cell<bool>,
        pub token: RefCell<Option<String>>,
        pub tracklist: RefCell<Option<Tracklist>>,
        pub visualizers: RefCell<Vec<Rc<Visualizer>>>,
        /// Tick handle that decays the waveform after audio stops.
        pub decay_tick: RefCell<Option<gtk::TickCallbackId>>,
        pub chips: RefCell<Vec<(Channel, gtk::Button)>>,
        /// Ticks once a second to advance the mix progress bar.
        pub progress_tick: RefCell<Option<glib::SourceId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FriskyWindow {
        const NAME: &'static str = "FriskyWindow";
        type Type = super::FriskyWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for FriskyWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup();
        }
    }

    impl WidgetImpl for FriskyWindow {}
    impl WindowImpl for FriskyWindow {}
    impl ApplicationWindowImpl for FriskyWindow {}
    impl AdwApplicationWindowImpl for FriskyWindow {}
}

glib::wrapper! {
    pub struct FriskyWindow(ObjectSubclass<imp::FriskyWindow>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl FriskyWindow {
    pub fn new(app: &adw::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    fn setup(&self) {
        let window = self.downgrade();
        self.imp()
            .view_stack
            .connect_visible_child_name_notify(move |stack| {
                let name = stack.visible_child_name();
                debug!("layout switched to {:?}", name);
                if let Some(window) = window.upgrade() {
                    window.set_compact_styling(name.as_deref() == Some("compact"));
                }
            });

        *self.imp().tracklist.borrow_mut() = Some(Tracklist::new(self.imp().tracklist_row.get()));
        self.build_visualizers();
        self.build_compact_chips();
        self.build_channel_pills();
        self.setup_actions();
        self.setup_volume();
    }

    /// Injects the pieces that need a running tokio runtime. Called once by the
    /// application after the window is constructed.
    pub fn attach(
        &self,
        client: FriskyClient,
        player: std::rc::Rc<Player>,
        refresh: RefreshHandle,
        events: Receiver,
        settings: gio::Settings,
    ) {
        let imp = self.imp();
        *imp.client.borrow_mut() = Some(client);
        *imp.player.borrow_mut() = Some(player);
        *imp.refresh.borrow_mut() = Some(refresh);
        *imp.settings.borrow_mut() = Some(settings);

        self.restore_state();
        self.spawn_event_loop(events);
    }

    // ---------------------------------------------------------------- setup

    fn build_channel_pills(&self) {
        let imp = self.imp();
        let mut pills = Vec::new();

        for channel in Channel::ALL {
            let pill = ChannelPill::new(channel);
            let window = self.downgrade();
            pill.connect_selected(move |channel| {
                if let Some(window) = window.upgrade() {
                    window.select_channel(channel, true);
                }
            });
            imp.channel_box.append(pill.widget());
            pills.push(pill);
        }

        *imp.pills.borrow_mut() = pills;
        self.update_pill_selection();
        self.update_chip_selection();
    }

    fn setup_actions(&self) {
        let toggle = gio::ActionEntry::builder("toggle-playback")
            .activate(|window: &Self, _, _| window.toggle_playback())
            .build();

        let refresh = gio::ActionEntry::builder("refresh")
            .activate(|window: &Self, _, _| {
                if let Some(handle) = window.imp().refresh.borrow().as_ref() {
                    handle.request();
                    window.toast("Refreshing…");
                }
            })
            .build();

        let preferences = gio::ActionEntry::builder("preferences")
            .activate(|window: &Self, _, _| window.present_preferences())
            .build();

        let next_channel = gio::ActionEntry::builder("next-channel")
            .activate(|window: &Self, _, _| window.step_channel(1))
            .build();

        let compact = gio::ActionEntry::builder("compact")
            .activate(|window: &Self, _, _| window.toggle_compact())
            .build();

        self.add_action_entries([toggle, refresh, preferences, compact, next_channel]);
    }

    /// Builds a visualiser for each layout.
    ///
    /// The cover one overlays the artwork and fades out on hover so the art is
    /// never permanently obscured. The compact one sits in the bar with nothing
    /// behind it, so it has no fade.
    fn build_visualizers(&self) {
        let imp = self.imp();

        let cover = Rc::new(Visualizer::new(VisualizerSize::COVER));
        imp.artwork_overlay.add_overlay(cover.widget());

        let hover = gtk::EventControllerMotion::new();
        let faded = cover.clone();
        hover.connect_enter(move |_, _, _| faded.set_faded(true));
        let restored = cover.clone();
        hover.connect_leave(move |_| restored.set_faded(false));
        imp.artwork_overlay.add_controller(hover);

        // The mini player's visualiser is a backdrop layer spanning the whole
        // window, not something drawn over the thumbnail. It sits at low
        // opacity behind the controls, so there is nothing to reveal on hover.
        let compact = Rc::new(Visualizer::new(VisualizerSize::COMPACT_BACKDROP));
        compact.widget().set_hexpand(true);
        compact.widget().set_vexpand(true);
        imp.compact_visualizer_slot.append(compact.widget());

        *imp.visualizers.borrow_mut() = vec![cover, compact];
    }

    /// In mini mode the gradient fills the window rather than sitting in a
    /// floating card, so the styling moves onto the window itself.
    fn set_compact_styling(&self, compact: bool) {
        if compact {
            self.add_css_class("compact-mode");
        } else {
            self.remove_css_class("compact-mode");
            // The breakpoint hides the header bar on the way in but nothing
            // restores it on the way out, so do it here.
            self.imp().toolbar_view.set_reveal_top_bars(true);
        }
        // Re-apply so the window picks up the current channel's gradient.
        self.update_channel_styling();
    }

    /// Puts the selected channel's gradient on everything that wears it.
    fn update_channel_styling(&self) {
        let imp = self.imp();
        let channel = imp.selected.get();

        for other in Channel::ALL {
            imp.play_button.remove_css_class(other.css_class());
            imp.compact_tint.remove_css_class(other.css_class());
            self.remove_css_class(other.css_class());
        }
        imp.play_button.add_css_class(channel.css_class());
        imp.compact_tint.add_css_class(channel.css_class());
        self.add_css_class(channel.css_class());
    }

    /// The four channel chips in the compact bar.
    ///
    /// Inline rather than behind a popover: at two characters each they fit,
    /// and switching channel is the one thing the mini player exists for.
    fn build_compact_chips(&self) {
        let imp = self.imp();
        let mut chips = Vec::new();

        for channel in Channel::ALL {
            let button = gtk::Button::builder()
                .label(channel.short_label())
                .tooltip_text(format!("Listen to {}", channel.title()))
                .valign(gtk::Align::Center)
                .css_classes(["compact-chip", channel.css_class()])
                .build();

            let window = self.downgrade();
            button.connect_clicked(move |_| {
                if let Some(window) = window.upgrade() {
                    window.select_channel(channel, true);
                }
            });

            imp.compact_chips.append(&button);
            chips.push((channel, button));
        }

        *imp.chips.borrow_mut() = chips;
    }

    fn update_chip_selection(&self) {
        let imp = self.imp();
        let selected = imp.selected.get();
        for (channel, button) in imp.chips.borrow().iter() {
            if *channel == selected {
                button.add_css_class("selected");
            } else {
                button.remove_css_class("selected");
            }
        }
    }

    /// Advances the mix progress bar once a second while something is playing.
    fn start_progress_tick(&self) {
        if self.imp().progress_tick.borrow().is_some() {
            return;
        }

        let window = self.downgrade();
        let tick = glib::timeout_add_seconds_local(1, move || {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            window.update_progress();
            glib::ControlFlow::Continue
        });
        *self.imp().progress_tick.borrow_mut() = Some(tick);
    }

    fn stop_progress_tick(&self) {
        if let Some(tick) = self.imp().progress_tick.borrow_mut().take() {
            tick.remove();
        }
    }

    /// Redraws the progress bar from the airing mix's schedule.
    fn update_progress(&self) {
        let imp = self.imp();

        let progress = self
            .current()
            .and_then(|entry| entry.progress_at(chrono::Utc::now()));

        match progress {
            Some(progress) => {
                imp.compact_progress.set_fraction(progress.fraction);
                imp.compact_time.set_label(&progress.label());
                imp.compact_progress.set_visible(true);
                imp.compact_time.set_visible(true);
            }
            // Without a schedule window there is nothing honest to show.
            None => {
                imp.compact_progress.set_visible(false);
                imp.compact_time.set_visible(false);
            }
        }
    }

    /// Snaps the window between the full and compact layouts.
    ///
    /// The breakpoint keys off height, so this is just a resize — no separate
    /// mode state to keep in sync with what the user does by dragging.
    fn toggle_compact(&self) {
        let compact = self.imp().view_stack.visible_child_name().as_deref() == Some("compact");
        if compact {
            self.set_default_size(420, 760);
        } else {
            self.set_default_size(470, 96);
        }
    }

    /// Runs the waveform down to silence after playback stops.
    fn start_decay(&self) {
        let imp = self.imp();
        if imp.decay_tick.borrow().is_some() {
            return;
        }

        let window = self.downgrade();
        let tick = self.add_tick_callback(move |_, _| {
            let Some(window) = window.upgrade() else {
                return glib::ControlFlow::Break;
            };
            // Every layout must decay on every frame, so this cannot be an
            // `any()` — that would short-circuit and leave later visualisers
            // frozen mid-waveform.
            let mut still_moving = false;
            for visualizer in window.imp().visualizers.borrow().iter() {
                still_moving |= visualizer.decay();
            }

            if still_moving {
                glib::ControlFlow::Continue
            } else {
                // Settled: drop the handle so a later stop can restart it.
                window.imp().decay_tick.borrow_mut().take();
                glib::ControlFlow::Break
            }
        });
        *imp.decay_tick.borrow_mut() = Some(tick);
    }

    /// Cancels any decay in progress, because fresh audio is arriving.
    fn stop_decay(&self) {
        if let Some(tick) = self.imp().decay_tick.borrow_mut().take() {
            tick.remove();
        }
    }

    /// Marks the active pill and dims the rest.
    fn update_pill_selection(&self) {
        let imp = self.imp();
        let selected = imp.selected.get();
        for pill in imp.pills.borrow().iter() {
            pill.set_selected(pill.channel() == selected);
        }
    }

    fn setup_volume(&self) {
        let imp = self.imp();
        imp.volume_button.set_icons(&[
            "audio-volume-muted-symbolic",
            "audio-volume-high-symbolic",
            "audio-volume-low-symbolic",
            "audio-volume-medium-symbolic",
        ]);

        let window = self.downgrade();
        imp.volume_button.connect_value_changed(move |_, value| {
            let Some(window) = window.upgrade() else {
                return;
            };
            let imp = window.imp();
            if let Some(player) = imp.player.borrow().as_ref() {
                player.set_volume(value);
            }
            if !imp.loading_volume.get() {
                if let Some(settings) = imp.settings.borrow().as_ref() {
                    let _ = settings.set_double("volume", value);
                }
            }
        });
    }

    fn restore_state(&self) {
        let imp = self.imp();
        let Some(settings) = imp.settings.borrow().clone() else {
            return;
        };

        let channel = Channel::from_id(&settings.string("last-channel")).unwrap_or_default();
        // Restore the selection without starting playback — an app that begins
        // making noise on launch is hostile.
        self.select_channel(channel, false);

        imp.loading_volume.set(true);
        let volume = settings.double("volume").clamp(0.0, 1.0);
        imp.volume_button.set_value(volume);
        if let Some(player) = imp.player.borrow().as_ref() {
            player.set_volume(volume);
        }
        imp.loading_volume.set(false);
    }

    // ------------------------------------------------------------- playback

    fn quality(&self) -> Quality {
        self.imp()
            .settings
            .borrow()
            .as_ref()
            .map(|s| Quality::from_nick(&s.string("quality")))
            .unwrap_or_default()
    }

    pub fn set_token(&self, token: Option<String>) {
        *self.imp().token.borrow_mut() = token;
    }

    fn toggle_playback(&self) {
        let imp = self.imp();
        let Some(player) = imp.player.borrow().clone() else {
            return;
        };

        if player.is_active() {
            player.stop();
        } else {
            self.start_playback();
        }
    }

    fn start_playback(&self) {
        let imp = self.imp();
        let Some(player) = imp.player.borrow().clone() else {
            return;
        };

        let channel = imp.selected.get();
        let quality = self.quality();
        let token = imp.token.borrow().clone();

        // Premium mounts 401 without an entitled token, so verify first and
        // fall back rather than failing the stream.
        if quality.requires_subscription() {
            let Some(client) = imp.client.borrow().clone() else {
                return;
            };
            let Some(token) = token else {
                self.toast("Log in to use higher audio quality. Playing at 96 kbps.");
                player.play(channel, Quality::Low, None);
                return;
            };

            let window = self.downgrade();
            glib::spawn_future_local(async move {
                let entitled = crate::auth::is_entitled(&client, &token, channel, quality).await;
                let Some(window) = window.upgrade() else {
                    return;
                };
                let Some(player) = window.imp().player.borrow().clone() else {
                    return;
                };

                if entitled {
                    player.play(channel, quality, Some(token));
                } else {
                    window.toast(&format!(
                        "{} needs a FRISKY subscription. Playing at 96 kbps.",
                        quality.label()
                    ));
                    player.play(channel, Quality::Low, None);
                }
            });
            return;
        }

        player.play(channel, quality, None);
    }

    fn select_channel(&self, channel: Channel, play: bool) {
        let imp = self.imp();
        let changed = imp.selected.get() != channel;
        imp.selected.set(channel);

        self.update_pill_selection();
        self.update_chip_selection();
        self.update_now_playing_display();

        if let Some(settings) = imp.settings.borrow().as_ref() {
            let _ = settings.set_string("last-channel", channel.id());
        }

        // Clicking a channel while another is playing switches to it; clicking
        // the one already playing does not restart it.
        let playing = imp
            .player
            .borrow()
            .as_ref()
            .map(|p| p.is_active())
            .unwrap_or(false);

        if play && (changed || !playing) {
            self.start_playback();
        }
    }

    // ---------------------------------------------------------------- events

    fn spawn_event_loop(&self, events: Receiver) {
        let window = self.downgrade();
        glib::spawn_future_local(async move {
            while let Ok(event) = events.recv().await {
                let Some(window) = window.upgrade() else {
                    break;
                };
                window.handle_event(event);
            }
            debug!("event loop finished");
        });
    }

    fn handle_event(&self, event: AppEvent) {
        match event {
            AppEvent::NowPlaying(entries) => self.on_now_playing(entries),
            AppEvent::Artwork { show_id, bytes } => self.on_artwork(show_id, &bytes),
            AppEvent::PlayerState(state) => self.on_player_state(state),
            AppEvent::IcyTitle(title) => {
                // The stream's own title is the fastest signal that the mix
                // changed; ask for fresh metadata to match it.
                debug!("ICY title: {title}");
                if let Some(handle) = self.imp().refresh.borrow().as_ref() {
                    handle.request();
                }
            }
            AppEvent::Level(level) => {
                for visualizer in self.imp().visualizers.borrow().iter() {
                    visualizer.push(level);
                }
            }
            AppEvent::Schedule(_) => {}
            AppEvent::Error(message) => self.toast(&message),
        }
    }

    fn on_now_playing(&self, entries: Vec<NowPlaying>) {
        let imp = self.imp();
        let previous_show = imp.displayed_show.get();

        {
            let mut map = imp.now_playing.borrow_mut();
            for entry in &entries {
                map.insert(entry.channel, entry.clone());
            }
        }

        for pill in imp.pills.borrow().iter() {
            let text = imp
                .now_playing
                .borrow()
                .get(&pill.channel())
                .map(|n| n.display_title());
            pill.set_now_playing(text.as_deref());
        }

        self.update_now_playing_display();
        self.update_progress();
        self.request_artwork();

        // Only notify once a mix has actually replaced another one, so the
        // first load after launch stays quiet.
        if let (Some(previous), Some(current)) = (previous_show, imp.displayed_show.get()) {
            if previous != current {
                self.notify_now_playing();
            }
        }

        crate::mpris::notify_changed();
    }

    fn update_now_playing_display(&self) {
        let imp = self.imp();
        let channel = imp.selected.get();
        let now_playing = imp.now_playing.borrow().get(&channel).cloned();

        imp.window_title.set_subtitle(channel.title());

        match now_playing {
            Some(entry) => {
                imp.track_title.set_label(&entry.display_title());
                self.update_progress();
                imp.track_subtitle.set_label(&entry.subtitle());
                imp.compact_title.set_label(&entry.display_title());
                imp.displayed_show.set(entry.show_id);

                let has_tracks = imp
                    .tracklist
                    .borrow()
                    .as_ref()
                    .map(|list| list.populate(&entry.mix.track_list))
                    .unwrap_or(false);
                imp.tracklist_group.set_visible(has_tracks);
            }
            None => {
                imp.track_title.set_label(channel.title());
                imp.track_subtitle.set_label(channel.tagline());
                imp.compact_title.set_label(channel.title());
                imp.tracklist_group.set_visible(false);
            }
        }

        // The play button wears the selected channel's gradient.
        self.update_channel_styling();
    }

    fn request_artwork(&self) {
        let imp = self.imp();
        let Some(show_id) = imp
            .now_playing
            .borrow()
            .get(&imp.selected.get())
            .and_then(|n| n.show_id)
        else {
            return;
        };
        let Some(client) = imp.client.borrow().clone() else {
            return;
        };
        let cache = imp.artwork_cache.clone();
        let Some(events) = crate::app::event_sender() else {
            return;
        };

        crate::runtime().spawn(async move {
            cache.request(&client, &events, show_id).await;
        });
    }

    fn on_artwork(&self, show_id: u64, bytes: &[u8]) {
        let imp = self.imp();
        // Artwork for a channel the user has since navigated away from.
        if imp.displayed_show.get() != Some(show_id) {
            return;
        }

        match gdk::Texture::from_bytes(&glib::Bytes::from(bytes)) {
            Ok(texture) => {
                imp.artwork.set_paintable(Some(&texture));
                imp.compact_artwork.set_paintable(Some(&texture));
                imp.compact_backdrop_art.set_paintable(Some(&texture));
                imp.artwork_overlay.add_css_class("has-art");
            }
            Err(error) => warn!("could not decode artwork for show {show_id}: {error}"),
        }
    }

    fn on_player_state(&self, state: PlayerState) {
        let imp = self.imp();

        match state {
            PlayerState::Buffering => {
                for visualizer in imp.visualizers.borrow().iter() {
                    visualizer.reset_range();
                }
                imp.play_stack.set_visible_child_name("busy");
                imp.play_spinner.set_spinning(true);
                imp.play_button.set_tooltip_text(Some("Connecting…"));
            }
            PlayerState::Playing => {
                self.stop_decay();
                self.start_progress_tick();
                imp.play_spinner.set_spinning(false);
                imp.play_stack.set_visible_child_name("idle");
                imp.play_icon
                    .set_icon_name(Some("media-playback-stop-symbolic"));
                imp.compact_play_icon
                    .set_icon_name(Some("media-playback-stop-symbolic"));
                imp.play_button.set_tooltip_text(Some("Stop"));
                imp.compact_play_button.set_tooltip_text(Some("Stop"));
            }
            PlayerState::Stopped => {
                self.stop_progress_tick();
                self.start_decay();
                imp.play_spinner.set_spinning(false);
                imp.play_stack.set_visible_child_name("idle");
                imp.play_icon
                    .set_icon_name(Some("media-playback-start-symbolic"));
                imp.compact_play_icon
                    .set_icon_name(Some("media-playback-start-symbolic"));
                imp.play_button.set_tooltip_text(Some("Play"));
                imp.compact_play_button.set_tooltip_text(Some("Play"));
            }
        }

        // Keep the window's own channel selection in step when playback was
        // started from MPRIS rather than the UI.
        if let Some(channel) = imp
            .player
            .borrow()
            .as_ref()
            .and_then(|p| p.current_channel())
        {
            if channel != imp.selected.get() && state != PlayerState::Stopped {
                self.select_channel(channel, false);
            }
        }

        crate::mpris::notify_changed();
    }

    // ----------------------------------------------------------- presentation

    pub fn toast(&self, message: &str) {
        self.imp().toast_overlay.add_toast(adw::Toast::new(message));
    }

    /// Current now-playing for the selected channel, for MPRIS and
    /// notifications.
    pub fn current(&self) -> Option<NowPlaying> {
        let imp = self.imp();
        imp.now_playing.borrow().get(&imp.selected.get()).cloned()
    }

    pub fn selected_channel(&self) -> Channel {
        self.imp().selected.get()
    }

    // Entry points for MPRIS, which drives the same actions as the UI but is
    // not a GTK widget and so cannot reach the private helpers.

    /// Switches channel and starts playing it.
    pub fn activate_channel(&self, channel: Channel) {
        self.select_channel(channel, true);
    }

    /// Moves `offset` channels from the current one, wrapping around.
    ///
    /// Shared by the transport's next button and MPRIS Next/Previous, so the
    /// two cannot drift apart.
    pub fn step_channel(&self, offset: isize) {
        let current = self.imp().selected.get();
        let count = Channel::ALL.len() as isize;
        let index = Channel::ALL.iter().position(|c| *c == current).unwrap_or(0) as isize;

        let next = Channel::ALL[(index + offset).rem_euclid(count) as usize];
        self.select_channel(next, true);
    }

    pub fn toggle_playback_external(&self) {
        self.toggle_playback();
    }

    pub fn start_playback_external(&self) {
        if !self
            .imp()
            .player
            .borrow()
            .as_ref()
            .map(|p| p.is_active())
            .unwrap_or(false)
        {
            self.start_playback();
        }
    }

    /// Applies a volume set from outside; the widget's handler persists it.
    pub fn set_volume_external(&self, volume: f64) {
        self.imp().volume_button.set_value(volume.clamp(0.0, 1.0));
    }

    fn notify_now_playing(&self) {
        let enabled = self
            .imp()
            .settings
            .borrow()
            .as_ref()
            .map(|settings| settings.boolean("notify-mix-change"))
            .unwrap_or(true);
        if !enabled {
            return;
        }

        // Only worth interrupting for if the user is actually listening.
        let playing = self
            .imp()
            .player
            .borrow()
            .as_ref()
            .map(|p| p.is_active())
            .unwrap_or(false);
        if !playing {
            return;
        }
        let Some(entry) = self.current() else { return };
        crate::app::send_notification(&entry);
    }

    fn present_preferences(&self) {
        crate::preferences::present(self);
    }
}
