//! Desktop-shell capabilities that cannot be inferred from an individual
//! window's dimensions.

use std::ffi::OsStr;

pub fn is_mobile_session() -> bool {
    [
        std::env::var_os("XDG_CURRENT_DESKTOP"),
        std::env::var_os("XDG_SESSION_DESKTOP"),
        std::env::var_os("DESKTOP_SESSION"),
    ]
    .iter()
    .any(|value| desktop_name_is_mobile(value.as_deref()))
}

fn desktop_name_is_mobile(value: Option<&OsStr>) -> bool {
    value
        .and_then(OsStr::to_str)
        .into_iter()
        .flat_map(|value| value.split([':', ';']))
        .any(|name| {
            matches!(
                name.trim().to_ascii_lowercase().as_str(),
                "phosh" | "gnome-mobile" | "plasma-mobile"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_phosh_in_a_composite_desktop_name() {
        assert!(desktop_name_is_mobile(Some(OsStr::new("Phosh:GNOME"))));
        assert!(desktop_name_is_mobile(Some(OsStr::new("plasma-mobile"))));
    }

    #[test]
    fn ordinary_gnome_remains_a_desktop_session() {
        assert!(!desktop_name_is_mobile(Some(OsStr::new("GNOME"))));
        assert!(!desktop_name_is_mobile(None));
    }
}
