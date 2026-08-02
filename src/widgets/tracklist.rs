//! The tracklist for the mix currently on air.
//!
//! The API returns the tracks of a mix but no per-track timing, and live radio
//! carries no position information, so there is no way to tell which track is
//! playing right now. The list is therefore shown whole and unhighlighted —
//! claiming a "current" row would be a guess.

use adw::prelude::*;
use gtk::glib;
use std::cell::RefCell;

use crate::api::model::Track;

/// Owns the rows it adds to an `AdwExpanderRow`.
///
/// `AdwExpanderRow` has no "remove everything", and the rows you add are not its
/// direct children — they live inside an internal list box — so walking the
/// widget tree to find them hands `remove()` widgets it rejects. Remembering
/// what we added is both simpler and correct.
pub struct Tracklist {
    row: adw::ExpanderRow,
    rows: RefCell<Vec<adw::ActionRow>>,
}

impl Tracklist {
    pub fn new(row: adw::ExpanderRow) -> Self {
        Self {
            row,
            rows: RefCell::new(Vec::new()),
        }
    }

    /// Replaces the contents with `tracks`, reporting whether there is anything
    /// worth showing.
    pub fn populate(&self, tracks: &[Track]) -> bool {
        self.clear();

        if tracks.is_empty() {
            self.row.set_subtitle("");
            return false;
        }

        let mut added = Vec::with_capacity(tracks.len());
        for (index, track) in tracks.iter().enumerate() {
            let row = build_row(index + 1, track);
            self.row.add_row(&row);
            added.push(row);
        }
        *self.rows.borrow_mut() = added;

        self.row.set_subtitle(&match tracks.len() {
            1 => "1 track".to_owned(),
            count => format!("{count} tracks"),
        });
        true
    }

    fn clear(&self) {
        for row in self.rows.borrow_mut().drain(..) {
            self.row.remove(&row);
        }
    }
}

fn build_row(position: usize, track: &Track) -> adw::ActionRow {
    let (title, subtitle) = describe(track);

    let row = adw::ActionRow::builder()
        .title(glib::markup_escape_text(&title))
        .subtitle(glib::markup_escape_text(&subtitle))
        .build();

    let index = gtk::Label::builder()
        .label(position.to_string())
        .valign(gtk::Align::Center)
        .xalign(1.0)
        .build();
    index.add_css_class("tracklist-index");
    row.add_prefix(&index);

    row
}

/// Splits a track into a title line and an artist line, tolerating the blanks
/// the API leaves in either field.
fn describe(track: &Track) -> (String, String) {
    let title = track.title.trim();
    let artist = track.artist.trim();

    match (title.is_empty(), artist.is_empty()) {
        (false, _) => (title.to_owned(), artist.to_owned()),
        // Artist only: better to show it as the title than an empty row.
        (true, false) => (artist.to_owned(), String::new()),
        (true, true) => ("Unknown track".to_owned(), String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str) -> Track {
        Track {
            title: title.into(),
            artist: artist.into(),
        }
    }

    #[test]
    fn splits_title_and_artist() {
        assert_eq!(
            describe(&track("Blue Sky", "Shakespears Sister")),
            ("Blue Sky".to_owned(), "Shakespears Sister".to_owned())
        );
    }

    #[test]
    fn promotes_the_artist_when_the_title_is_blank() {
        assert_eq!(
            describe(&track("", "Some Artist")),
            ("Some Artist".to_owned(), String::new())
        );
    }

    #[test]
    fn labels_wholly_empty_entries() {
        assert_eq!(
            describe(&track("  ", "")),
            ("Unknown track".to_owned(), String::new())
        );
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            describe(&track("  Padded  ", "  Artist ")),
            ("Padded".to_owned(), "Artist".to_owned())
        );
    }
}
