//! Preferences: audio quality and the FRISKY account used to unlock it.

use adw::prelude::*;
use gtk::{gio, glib};
use tracing::warn;

use crate::channel::Quality;
use crate::window::FriskyWindow;

pub fn present(window: &FriskyWindow) {
    let settings = gio::Settings::new(crate::app::APP_ID);

    let dialog = adw::PreferencesDialog::builder()
        .title("Preferences")
        .build();
    dialog.add(&audio_page(window, &settings));
    adw::prelude::AdwDialogExt::present(&dialog, Some(window));
}

fn audio_page(window: &FriskyWindow, settings: &gio::Settings) -> adw::PreferencesPage {
    let page = adw::PreferencesPage::builder()
        .title("Audio")
        .icon_name("audio-headphones-symbolic")
        .build();

    page.add(&quality_group(settings));
    page.add(&notification_group(settings));
    page.add(&account_group(window));
    page
}

fn notification_group(settings: &gio::Settings) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Notifications")
        .build();

    let row = adw::SwitchRow::builder()
        .title("Mix Changes")
        .subtitle(
            "Notify when a new mix starts. Live radio has no per-track \
             information, so this cannot fire per track.",
        )
        .build();

    settings.bind("notify-mix-change", &row, "active").build();

    group.add(&row);
    group
}

fn quality_group(settings: &gio::Settings) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("Audio Quality")
        .description(
            "FRISKY streams 96 kbps free of charge. Higher bitrates need a \
             subscription and an account logged in below.",
        )
        .build();

    let labels: Vec<String> = Quality::ALL
        .iter()
        .map(|quality| {
            let suffix = if quality.requires_subscription() {
                " · Premium"
            } else {
                ""
            };
            format!("{} ({}){}", quality.label(), quality.bitrate(), suffix)
        })
        .collect();

    let model = gtk::StringList::new(&labels.iter().map(String::as_str).collect::<Vec<_>>());
    let row = adw::ComboRow::builder()
        .title("Quality")
        .model(&model)
        .build();

    let stored = Quality::from_nick(&settings.string("quality"));
    let selected = Quality::ALL.iter().position(|q| *q == stored).unwrap_or(0) as u32;
    row.set_selected(selected);

    let settings = settings.clone();
    row.connect_selected_notify(move |row| {
        let index = row.selected() as usize;
        if let Some(quality) = Quality::ALL.get(index) {
            let _ = settings.set_string("quality", quality.nick());
        }
    });

    group.add(&row);
    group
}

fn account_group(window: &FriskyWindow) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title("FRISKY Account")
        .description("Only needed for 128 kbps and 320 kbps streams.")
        .build();

    let row = adw::ActionRow::builder()
        .title("Account")
        .subtitle("Checking…")
        .build();

    let button = gtk::Button::builder()
        .label("Log In…")
        .valign(gtk::Align::Center)
        .build();
    row.add_suffix(&button);
    group.add(&row);

    // Reflect whether a token is already stored.
    let row_ref = row.clone();
    let button_ref = button.clone();
    glib::spawn_future_local(async move {
        let stored = crate::runtime()
            .spawn(async { crate::auth::stored_token().await })
            .await
            .ok()
            .flatten();
        set_logged_in(&row_ref, &button_ref, stored.is_some());
    });

    let window = window.downgrade();
    let row_ref = row.clone();
    button.connect_clicked(move |button| {
        let Some(window) = window.upgrade() else {
            return;
        };
        if button.label().as_deref() == Some("Log Out") {
            let row = row_ref.clone();
            let button = button.clone();
            let window = window.clone();
            glib::spawn_future_local(async move {
                let cleared = crate::runtime()
                    .spawn(async { crate::auth::clear_token().await })
                    .await;
                match cleared {
                    Ok(Ok(())) => {
                        window.set_token(None);
                        set_logged_in(&row, &button, false);
                        window.toast("Logged out.");
                    }
                    _ => window.toast("Could not remove the stored token."),
                }
            });
        } else {
            present_login(&window, &row_ref, button);
        }
    });

    group
}

fn set_logged_in(row: &adw::ActionRow, button: &gtk::Button, logged_in: bool) {
    if logged_in {
        row.set_subtitle("Logged in");
        button.set_label("Log Out");
    } else {
        row.set_subtitle("Not logged in — streaming at 96 kbps");
        button.set_label("Log In…");
    }
}

fn present_login(window: &FriskyWindow, row: &adw::ActionRow, button: &gtk::Button) {
    let email = adw::EntryRow::builder().title("Email").build();
    let password = adw::PasswordEntryRow::builder().title("Password").build();

    let group = adw::PreferencesGroup::new();
    group.add(&email);
    group.add(&password);

    let dialog = adw::AlertDialog::builder()
        .heading("Log in to FRISKY")
        .body("Your credentials go straight to FRISKY. Only the returned token is kept, in the system keyring.")
        .extra_child(&group)
        .build();
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("login", "Log In");
    dialog.set_response_appearance("login", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("login"));
    dialog.set_close_response("cancel");

    let parent = window.clone();
    let window = window.clone();
    let row = row.clone();
    let button = button.clone();
    dialog.connect_response(None, move |dialog, response| {
        if response != "login" {
            return;
        }
        let email = email.text().trim().to_owned();
        let password = password.text().to_string();

        if email.is_empty() || password.is_empty() {
            window.toast("Enter both an email address and a password.");
            return;
        }
        dialog.close();

        let window = window.clone();
        let row = row.clone();
        let button = button.clone();
        glib::spawn_future_local(async move {
            match log_in(email, password).await {
                Ok(()) => {
                    let token = crate::runtime()
                        .spawn(async { crate::auth::stored_token().await })
                        .await
                        .ok()
                        .flatten();
                    window.set_token(token);
                    set_logged_in(&row, &button, true);
                    window.toast("Logged in.");
                }
                Err(error) => {
                    warn!("login failed: {error:#}");
                    window.toast("Log in failed. Check your email and password.");
                }
            }
        });
    });

    adw::prelude::AdwDialogExt::present(&dialog, Some(&parent));
}

/// Exchanges credentials for a token and stores it, entirely off the main
/// thread.
async fn log_in(email: String, password: String) -> anyhow::Result<()> {
    crate::runtime()
        .spawn(async move {
            let client = crate::api::FriskyClient::new()?;
            let token = client.login(&email, &password).await?;
            crate::auth::store_token(&token).await?;
            anyhow::Ok(())
        })
        .await?
}
