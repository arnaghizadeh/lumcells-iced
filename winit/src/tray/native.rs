//! The tray icon on macOS and Windows: the `tray-icon` crate. It has to be
//! made, changed and dropped on the thread of the event loop.
use crate::runtime::tray::{Event, Icon, Item, Menu, Settings};

use tray_icon::menu::{CheckMenuItem, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use std::sync::Once;

pub struct Tray {
    raw: TrayIcon,
    settings: Settings,
}

impl Tray {
    pub fn new(settings: Settings) -> Option<Self> {
        static HANDLERS: Once = Once::new();

        HANDLERS.call_once(|| {
            MenuEvent::set_event_handler(Some(|event: MenuEvent| {
                super::emit(Event::MenuItem(event.id.0));
            }));

            TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    super::emit(Event::Click);
                }
            }));
        });

        // Windows draws the menu of a program light unless the program
        // says that it knows the dark look
        #[cfg(target_os = "windows")]
        windows::menus_follow_the_system();

        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu(&settings.menu)))
            // On macOS a click on a status item opens its menu; on Windows
            // the main button activates and the other one opens the menu
            .with_menu_on_left_click(cfg!(target_os = "macos"));

        if let Some(icon) = icon(&settings.icon) {
            builder = builder.with_icon(icon);
        }

        let first = first_tooltip(
            cfg!(target_os = "windows"),
            &settings.name,
            &settings.tooltip,
        );

        if !first.is_empty() {
            builder = builder.with_tooltip(first);
        }

        let raw = match builder.build() {
            Ok(raw) => raw,
            Err(error) => {
                log::error!("the tray icon could not be made: {error}");
                return None;
            }
        };

        // The status follows the name at once
        if first != settings.tooltip
            && let Err(error) = raw.set_tooltip(Some(&settings.tooltip))
        {
            log::error!("the tray tooltip could not be set: {error}");
        }

        let tray = Self { raw, settings };
        tray.name();

        Some(tray)
    }

    /// Tells assistive technology what the icon is called.
    fn name(&self) {
        // On Windows the name is the first tooltip (see `first_tooltip`)
        #[cfg(target_os = "macos")]
        macos::name(&self.raw, self.settings.title());
    }

    pub fn update(&mut self, settings: Settings) {
        if settings.icon != self.settings.icon
            && let Err(error) = self.raw.set_icon(icon(&settings.icon))
        {
            log::error!("the tray icon could not be changed: {error}");
        }

        if settings.tooltip != self.settings.tooltip
            && let Err(error) = self.raw.set_tooltip(
                (!settings.tooltip.is_empty()).then_some(&settings.tooltip),
            )
        {
            log::error!("the tray tooltip could not be changed: {error}");
        }

        if settings.menu != self.settings.menu {
            self.raw.set_menu(Some(Box::new(menu(&settings.menu))));
        }

        let is_renamed = settings.title() != self.settings.title();

        self.settings = settings;

        if is_renamed {
            self.name();
        }
    }
}

/// The tooltip an icon is made with.
///
/// Windows keeps the first tooltip it sees for an icon as the name of the
/// icon: in Settings, Personalization, Taskbar, "Other system tray icons",
/// and as the start of what a screen reader says. A status there ("not
/// set up yet") would stay for good, so an icon with a name is first
/// shown with its name as the tooltip.
fn first_tooltip<'a>(
    is_windows: bool,
    name: &'a str,
    tooltip: &'a str,
) -> &'a str {
    if is_windows && !name.is_empty() {
        name
    } else {
        tooltip
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};

    use std::ffi::CString;

    /// Gives the button of the status item its accessibility title: what
    /// VoiceOver calls it. Without one the item has an empty title and
    /// its tooltip only as the help text.
    #[allow(unsafe_code)]
    pub fn name(tray: &tray_icon::TrayIcon, name: &str) {
        let Some(item) = tray.ns_status_item() else {
            return;
        };

        let Ok(name) = CString::new(name) else {
            return;
        };

        let item: *const AnyObject = (&raw const *item).cast();

        // SAFETY: on the thread of the event loop (the tray is made and
        // changed there). `button` of an NSStatusItem is an NSStatusBarButton
        // or nil, which is checked; the string is a valid, autoreleased
        // NSString or nil, and `setAccessibilityTitle:` takes either.
        unsafe {
            let button: *mut AnyObject = msg_send![item, button];

            if button.is_null() {
                return;
            }

            let title: *mut AnyObject = msg_send![class!(NSString), stringWithUTF8String: name.as_ptr()];

            let _: () = msg_send![button, setAccessibilityTitle: title];
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use windows_sys::Wdk::System::SystemServices::RtlGetVersion;
    use windows_sys::Win32::System::LibraryLoader::{
        GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
    };
    use windows_sys::Win32::System::SystemInformation::OSVERSIONINFOW;

    use std::sync::Once;

    /// Windows 10 1903: the first with `SetPreferredAppMode`.
    const FIRST_BUILD: u32 = 18362;

    /// Asks Windows to draw the menus of this program in the look of the
    /// system: dark when the system's look for apps is dark.
    ///
    /// A menu of Windows (what `TrackPopupMenu` shows: the menu of the
    /// tray icon) is drawn by Windows, light, unless the program says
    /// that it knows the dark look. There is no documented way to say so:
    /// this is `SetPreferredAppMode(AllowDark)`, the function number 135
    /// of `uxtheme.dll`, which Explorer and the programs of Windows use
    /// themselves, followed by `FlushMenuThemes` (136). Before Windows 10
    /// 1903 the number means something else, so nothing is asked there.
    ///
    /// The menu follows the system, not the theme of the program: a
    /// program that is dark by its own choice on a light system keeps a
    /// light menu, like every tray menu there.
    #[allow(unsafe_code)]
    pub fn menus_follow_the_system() {
        static ONCE: Once = Once::new();

        ONCE.call_once(|| {
            // SAFETY: `RtlGetVersion` fills a structure of the size it is
            // told. The library is loaded from the system directory only.
            // The two functions are called with the signatures they have
            // since the build that is checked for.
            unsafe {
                let mut version: OSVERSIONINFOW = std::mem::zeroed();
                version.dwOSVersionInfoSize =
                    size_of::<OSVERSIONINFOW>() as u32;

                if RtlGetVersion(&mut version) != 0
                    || !knows_the_dark_look(version.dwBuildNumber)
                {
                    return;
                }

                let name: Vec<u16> = "uxtheme.dll\0".encode_utf16().collect();

                let library = LoadLibraryExW(
                    name.as_ptr(),
                    std::ptr::null_mut(),
                    LOAD_LIBRARY_SEARCH_SYSTEM32,
                );

                if library.is_null() {
                    return;
                }

                // A function by its number: the number in place of the
                // pointer to a name
                const SET_PREFERRED_APP_MODE: usize = 135;
                const FLUSH_MENU_THEMES: usize = 136;
                const ALLOW_DARK: i32 = 1;

                if let Some(function) =
                    GetProcAddress(library, SET_PREFERRED_APP_MODE as *const u8)
                {
                    let set_preferred_app_mode: unsafe extern "system" fn(
                        i32,
                    )
                        -> i32 = std::mem::transmute(function);

                    let _ = set_preferred_app_mode(ALLOW_DARK);
                }

                if let Some(function) =
                    GetProcAddress(library, FLUSH_MENU_THEMES as *const u8)
                {
                    let flush_menu_themes: unsafe extern "system" fn() =
                        std::mem::transmute(function);

                    flush_menu_themes();
                }
            }
        });
    }

    /// Whether this build of Windows has `SetPreferredAppMode`.
    pub fn knows_the_dark_look(build: u32) -> bool {
        build >= FIRST_BUILD
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn only_windows_10_1903_and_later_are_asked() {
            assert!(!knows_the_dark_look(17763), "1809");
            assert!(knows_the_dark_look(18362), "1903");
            assert!(knows_the_dark_look(26300), "Windows 11");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_first_sees_the_name_and_the_others_the_tooltip() {
        assert_eq!(first_tooltip(true, "Drive", "Drive: Paused"), "Drive");
        assert_eq!(first_tooltip(true, "", "Drive: Paused"), "Drive: Paused");
        assert_eq!(
            first_tooltip(false, "Drive", "Drive: Paused"),
            "Drive: Paused"
        );
        assert_eq!(first_tooltip(true, "", ""), "");
    }
}

fn icon(icon: &Icon) -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(
        icon.rgba().to_vec(),
        icon.width(),
        icon.height(),
    )
    .map_err(|error| log::error!("the tray icon is not valid: {error}"))
    .ok()
}

fn menu(menu: &Menu) -> tray_icon::menu::Menu {
    let raw = tray_icon::menu::Menu::new();

    for item in &menu.items {
        let result = match item {
            Item::Button {
                id,
                label,
                enabled,
                checked: None,
            } => raw.append(&MenuItem::with_id(
                id.as_str(),
                label,
                *enabled,
                None,
            )),
            Item::Button {
                id,
                label,
                enabled,
                checked: Some(checked),
            } => raw.append(&CheckMenuItem::with_id(
                id.as_str(),
                label,
                *enabled,
                *checked,
                None,
            )),
            Item::Separator => raw.append(&PredefinedMenuItem::separator()),
        };

        if let Err(error) = result {
            log::error!("an item of the tray menu could not be added: {error}");
        }
    }

    raw
}
