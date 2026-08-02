//! One selectable channel, rendered in that channel's brand gradient.
//!
//! The gradient itself lives in `style.css`, keyed off the class returned by
//! [`Channel::css_class`]; this only assembles the widgets and keeps the
//! now-playing line current.

use gtk::prelude::*;

use crate::channel::Channel;

pub struct ChannelPill {
    button: gtk::Button,
    now_playing: gtk::Label,
    channel: Channel,
}

impl ChannelPill {
    pub fn new(channel: Channel) -> Self {
        let name = gtk::Label::builder()
            .label(channel.wordmark())
            .halign(gtk::Align::Start)
            .build();
        name.add_css_class("pill-name");

        // Starts as the channel's tagline and is replaced by what is on air.
        let now_playing = gtk::Label::builder()
            .label(channel.tagline())
            .halign(gtk::Align::Start)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(34)
            .single_line_mode(true)
            .build();
        now_playing.add_css_class("pill-nowplaying");

        let surface = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        surface.add_css_class("pill-surface");
        surface.append(&name);
        surface.append(&now_playing);

        let button = gtk::Button::builder()
            .child(&surface)
            .tooltip_text(format!("Listen to {}", channel.title()))
            .build();
        button.add_css_class("channel-pill");
        button.add_css_class(channel.css_class());
        button.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} channel",
            channel.title()
        ))]);

        Self {
            button,
            now_playing,
            channel,
        }
    }

    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }

    pub fn channel(&self) -> Channel {
        self.channel
    }

    /// Runs `handler` when this pill is activated.
    pub fn connect_selected<F: Fn(Channel) + 'static>(&self, handler: F) {
        let channel = self.channel;
        self.button.connect_clicked(move |_| handler(channel));
    }

    pub fn set_selected(&self, selected: bool) {
        if selected {
            self.button.add_css_class("selected");
        } else {
            self.button.remove_css_class("selected");
        }
        self.button
            .update_state(&[gtk::accessible::State::Selected(Some(selected))]);
    }

    /// Sets the second line to what is on air, falling back to the channel's
    /// tagline when nothing is known yet.
    pub fn set_now_playing(&self, text: Option<&str>) {
        let text = text
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(self.channel.tagline());
        self.now_playing.set_label(text);
        self.button
            .set_tooltip_text(Some(&format!("{} — {}", self.channel.title(), text)));
    }
}
