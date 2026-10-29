//! Windows notifications (toasts), e.g. when transfers are done.
//!
//! Windows only shows a program's toasts under its AppUserModelID: the
//! process sets [`APP_ID`] at start ([`set_app_id`]), and the installer's
//! Start-menu shortcut carries the same ID. Portable copies and dev builds
//! have no shortcut, so before the first toast the ID is registered for the
//! current user under `HKCU\Software\Classes\AppUserModelId\<id>` (name and
//! icon); the installed version does the same, which keeps the two alike.
//! The uninstaller removes it again ([`unregister`]).
//!
//! Clicking a toast while Oxplorer runs calls the `on_click` given to
//! [`show`]. Toasts from a window that has closed since do nothing more
//! than Windows does on its own.

use std::io;

/// The AppUserModelID of Oxplorer (also in `installer/oxplorer.iss`).
pub const APP_ID: &str = "Flory-1.Oxplorer";

/// Escape `text` for the toast's XML.
pub fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 has no other control characters.
            c if c.is_control() && !matches!(c, '\n' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

/// The toast's XML: a title and a body.
pub fn toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(body)
    )
}

/// A toast on screen (or in the notification centre). Keep it while its
/// click should still reach the program.
pub struct Toast {
    #[cfg(windows)]
    _toast: windows::UI::Notifications::ToastNotification,
}

/// Give this process Oxplorer's AppUserModelID. Call it at start, before
/// the window is made (the taskbar groups windows by it).
pub fn set_app_id() {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        // SAFETY: plain call with a valid string.
        if let Err(e) = unsafe {
            windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(
                APP_ID,
            ))
        } {
            log::warn!("could not set the AppUserModelID: {e}");
        }
    }
}

/// Show a toast with `title` and `body`; `on_click` runs (on another
/// thread) when it's clicked while Oxplorer runs. `icon_png` is the
/// program's icon, written next to the settings for Windows to show.
pub fn show(
    title: &str,
    body: &str,
    icon_png: &[u8],
    on_click: impl Fn() + Send + 'static,
) -> io::Result<Toast> {
    #[cfg(windows)]
    {
        imp::register_once(icon_png);
        imp::show(&toast_xml(title, body), on_click)
            .map(|t| Toast { _toast: t })
            .map_err(|e| io::Error::other(e.to_string()))
    }
    #[cfg(not(windows))]
    {
        let _ = (title, body, icon_png, on_click);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "only available on Windows",
        ))
    }
}

/// Remove the registration of [`APP_ID`] (for the uninstaller).
pub fn unregister() {
    #[cfg(windows)]
    crate::default_app::delete_class_key(&format!(r"AppUserModelId\{APP_ID}"));
}

#[cfg(windows)]
mod imp {
    use super::APP_ID;
    use std::sync::Once;
    use windows::core::{IInspectable, HSTRING};
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    /// Register the ID for the current user, once per run (the icon may
    /// have moved with a portable copy, so it's written each run).
    pub(super) fn register_once(icon_png: &[u8]) {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            let key = format!(r"AppUserModelId\{APP_ID}");
            let icon = crate::config_dir().join("notification-icon.png");
            let wrote = std::fs::create_dir_all(crate::config_dir())
                .and_then(|()| std::fs::write(&icon, icon_png));
            let result = crate::default_app::set_class_value(&key, "DisplayName", crate::APP_NAME)
                .and_then(|()| match wrote {
                    Ok(()) => crate::default_app::set_class_value(
                        &key,
                        "IconUri",
                        &icon.display().to_string(),
                    ),
                    Err(_) => Ok(()),
                });
            if let Err(e) = result {
                log::warn!("could not register for notifications: {e}");
            }
        });
    }

    pub(super) fn show(
        xml: &str,
        on_click: impl Fn() + Send + 'static,
    ) -> windows::core::Result<ToastNotification> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml))?;
        let toast = ToastNotification::CreateToastNotification(&doc)?;
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
            move |_, _| {
                on_click();
                Ok(())
            },
        ))?;
        ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID))?
            .Show(&toast)?;
        Ok(toast)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_is_escaped() {
        assert_eq!(
            xml_escape("a & <b> \"c\" 'd'"),
            "a &amp; &lt;b&gt; &quot;c&quot; &apos;d&apos;"
        );
        assert_eq!(xml_escape("bell\u{7} tab\t"), "bell tab\t");
        let xml = toast_xml("Copy finished", "R&D.txt to D:\\Backup");
        assert!(xml.contains("<text>Copy finished</text><text>R&amp;D.txt to D:\\Backup</text>"));
        assert!(xml.starts_with("<toast>") && xml.ends_with("</toast>"));
    }

    #[test]
    fn the_installer_shortcut_carries_the_app_id() {
        let iss = include_str!("../../../installer/oxplorer.iss");
        let id = format!("AppUserModelID: \"{APP_ID}\"");
        assert!(iss.contains(&format!(
            "Name: \"{{autoprograms}}\\Oxplorer\"; Filename: \"{{app}}\\oxplorer.exe\"; {id}"
        )));
    }
}
