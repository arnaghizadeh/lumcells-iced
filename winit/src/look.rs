//! Watch the look of the system (light or dark) where the window library
//! does not tell reliably.
//!
//! - macOS: a window that was given a look of its own (the shell gives
//!   each window the look of the program's theme) is no longer told of the
//!   system's changes. The system-wide notification
//!   `AppleInterfaceThemeChangedNotification` still comes; the look is
//!   then read from the settings of the system (`AppleInterfaceStyle`).
//! - Windows: the look is told with `WM_SETTINGCHANGE`, which arrives
//!   before the new value can be read: a program is one message behind.
//!   The value itself (`AppsUseLightTheme`) is watched in the registry.
//! - Elsewhere there is nothing to watch here (Linux: the
//!   `linux-theme-detection` feature).
use crate::core::theme;

/// The look of macOS as its settings name it: `AppleInterfaceStyle` is
/// "Dark", or not there at all for light.
pub fn of_macos(apple_interface_style: Option<&str>) -> theme::Mode {
    if apple_interface_style == Some("Dark") {
        theme::Mode::Dark
    } else {
        theme::Mode::Light
    }
}

/// The look of Windows for apps as its registry holds it:
/// `AppsUseLightTheme` is 0 for dark; 1, or not there at all (before
/// Windows 10 1607), for light.
pub fn of_windows(apps_use_light_theme: Option<u32>) -> theme::Mode {
    if apps_use_light_theme == Some(0) {
        theme::Mode::Dark
    } else {
        theme::Mode::Light
    }
}

/// Starts watching; `tell` is called with the look of the system whenever
/// it may have changed (on the main thread on macOS, on a thread of its
/// own on Windows). Has to be called on the thread of the event loop, once
/// it runs. Does nothing on other platforms.
pub fn watch(tell: impl Fn(theme::Mode) + Send + Sync + 'static) {
    #[cfg(target_os = "macos")]
    crate::platform::watch_look(move |style| tell(of_macos(style.as_deref())));

    #[cfg(target_os = "windows")]
    windows::watch(tell);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    drop(tell);
}

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
mod windows {
    use super::{of_windows, theme};

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, FALSE};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_QUERY_VALUE,
        REG_NOTIFY_CHANGE_LAST_SET, RRF_RT_REG_DWORD, RegCloseKey,
        RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// `AppsUseLightTheme` of the open key, if it is there.
    fn apps_use_light_theme(key: HKEY) -> Option<u32> {
        let name = wide("AppsUseLightTheme");
        let mut value = 0u32;
        let mut size = size_of::<u32>() as u32;

        // SAFETY: the key is open, the name ends with a zero, and the
        // value is written into four bytes that are told to be four.
        let status = unsafe {
            RegGetValueW(
                key,
                std::ptr::null(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&raw mut value).cast(),
                &raw mut size,
            )
        };

        (status == ERROR_SUCCESS).then_some(value)
    }

    pub fn watch(tell: impl Fn(theme::Mode) + Send + Sync + 'static) {
        let watcher = move || {
            let path = wide(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
            );
            let mut key: HKEY = std::ptr::null_mut();

            // SAFETY: the path ends with a zero; the key is closed below.
            let status = unsafe {
                RegOpenKeyExW(
                    HKEY_CURRENT_USER,
                    path.as_ptr(),
                    0,
                    KEY_NOTIFY | KEY_QUERY_VALUE,
                    &raw mut key,
                )
            };

            if status != ERROR_SUCCESS {
                log::warn!(
                    "the look of Windows is not watched: the registry \
                    key could not be opened ({status})"
                );
                return;
            }

            let mut last = apps_use_light_theme(key);

            loop {
                // Waits until a value of the key is written
                // SAFETY: the key is open; no event, not asynchronous.
                let status = unsafe {
                    RegNotifyChangeKeyValue(
                        key,
                        FALSE,
                        REG_NOTIFY_CHANGE_LAST_SET,
                        std::ptr::null_mut(),
                        FALSE,
                    )
                };

                if status != ERROR_SUCCESS {
                    break;
                }

                let now = apps_use_light_theme(key);

                if now != last {
                    last = now;
                    tell(of_windows(now));
                }
            }

            // SAFETY: the key was opened above and is not used after.
            let _ = unsafe { RegCloseKey(key) };
        };

        if let Err(error) = std::thread::Builder::new()
            .name("iced-system-look".to_owned())
            .spawn(watcher)
        {
            log::warn!("the look of Windows is not watched: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_is_dark_only_when_its_settings_say_dark() {
        assert_eq!(of_macos(Some("Dark")), theme::Mode::Dark);
        assert_eq!(of_macos(None), theme::Mode::Light);
        assert_eq!(of_macos(Some("Light")), theme::Mode::Light);
    }

    #[test]
    fn windows_is_dark_only_when_apps_do_not_use_the_light_theme() {
        assert_eq!(of_windows(Some(0)), theme::Mode::Dark);
        assert_eq!(of_windows(Some(1)), theme::Mode::Light);
        assert_eq!(of_windows(None), theme::Mode::Light);
    }
}
