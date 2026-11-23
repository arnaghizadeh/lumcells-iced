//! What is of macOS alone.
//!
//! Built by the CI of the fork on macOS; not run by a person yet.
use crate::runtime::shell::ActivationPolicy;

/// The activation policy as `winit` names it, for the start of the program.
pub fn activation_policy(
    policy: ActivationPolicy,
) -> winit::platform::macos::ActivationPolicy {
    use winit::platform::macos::ActivationPolicy as Raw;

    match policy {
        ActivationPolicy::Regular => Raw::Regular,
        ActivationPolicy::Accessory => Raw::Accessory,
        ActivationPolicy::Prohibited => Raw::Prohibited,
    }
}

/// Changes the activation policy of the running program:
/// `[NSApp setActivationPolicy:]`. Has to be called on the main thread
/// (the one of the event loop); elsewhere it does nothing.
pub fn set_activation_policy(policy: ActivationPolicy) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let Some(main_thread) = MainThreadMarker::new() else {
        log::warn!("the activation policy is set on the main thread only");
        return;
    };

    let raw = match policy {
        ActivationPolicy::Regular => NSApplicationActivationPolicy::Regular,
        ActivationPolicy::Accessory => NSApplicationActivationPolicy::Accessory,
        ActivationPolicy::Prohibited => {
            NSApplicationActivationPolicy::Prohibited
        }
    };

    let application = NSApplication::sharedApplication(main_thread);

    if !application.setActivationPolicy(raw) {
        log::warn!("macOS did not take the activation policy {policy:?}");
    }

    // A program that becomes regular while it opens a window should also
    // come to the front
    if policy == ActivationPolicy::Regular {
        #[allow(deprecated)]
        application.activateIgnoringOtherApps(true);
    }
}

pub use listener::{take_over_quit, watch_look};

/// What AppKit tells an object of the shell's own: "Quit" of the menu of
/// the application, and the look of the system changing.
///
/// The delegate of the application is `winit`'s and has to stay `winit`'s
/// (0.30 panics when another one is set), so nothing here is a delegate:
/// the menu item is given another target, and the look comes as a
/// notification.
#[allow(unsafe_code)]
mod listener {
    use objc2::runtime::{AnyObject, NSObject, Sel};
    use objc2::{
        ClassType, MainThreadMarker, class, define_class, msg_send, sel,
    };

    use std::ffi::{CStr, c_char};
    use std::sync::OnceLock;

    static QUIT: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

    #[allow(clippy::type_complexity)]
    static LOOK: OnceLock<Box<dyn Fn(Option<String>) + Send + Sync>> =
        OnceLock::new();

    define_class!(
        // SAFETY: NSObject has no requirements for its subclasses, and
        // the class has no instance variables and no `Drop`.
        #[unsafe(super(NSObject))]
        #[name = "IcedShellListener"]
        struct Listener;

        impl Listener {
            #[unsafe(method(icedQuitRequested:))]
            fn quit_requested(&self, _sender: *mut AnyObject) {
                if let Some(quit) = QUIT.get() {
                    quit();
                }
            }

            #[unsafe(method(icedLookChanged:))]
            fn look_changed(&self, _notification: *mut AnyObject) {
                if let Some(look) = LOOK.get() {
                    look(apple_interface_style());
                }
            }
        }
    );

    /// The one listener of the process; it is never freed.
    fn listener() -> *mut AnyObject {
        static LISTENER: OnceLock<usize> = OnceLock::new();

        *LISTENER.get_or_init(|| {
            // SAFETY: `new` of a subclass of NSObject without instance
            // variables gives an object that is owned here, and kept.
            let listener: *mut AnyObject =
                unsafe { msg_send![Listener::class(), new] };

            listener as usize
        }) as *mut AnyObject
    }

    /// An autoreleased NSString, or nil.
    unsafe fn string(text: &CStr) -> *mut AnyObject {
        // SAFETY: the text ends with a zero and is valid UTF-8 or the
        // result is nil.
        unsafe {
            msg_send![class!(NSString), stringWithUTF8String: text.as_ptr()]
        }
    }

    /// `AppleInterfaceStyle` of the settings of the system: "Dark", or
    /// nothing for light.
    fn apple_interface_style() -> Option<String> {
        // SAFETY: plain messages to NSUserDefaults; every object is
        // checked for nil before it is used, and the characters are copied
        // before anything else is done.
        unsafe {
            let defaults: *mut AnyObject =
                msg_send![class!(NSUserDefaults), standardUserDefaults];

            if defaults.is_null() {
                return None;
            }

            let style: *mut AnyObject = msg_send![
                defaults,
                stringForKey: string(c"AppleInterfaceStyle")
            ];

            if style.is_null() {
                return None;
            }

            let characters: *const c_char = msg_send![style, UTF8String];

            if characters.is_null() {
                return None;
            }

            Some(CStr::from_ptr(characters).to_string_lossy().into_owned())
        }
    }

    /// Makes "Quit" of the menu of the application (Command-Q) call
    /// `quit` instead of `terminate:`, which ends the process inside
    /// AppKit: the event loop never returns, nothing of the program is
    /// dropped. Returns whether the menu has such an item.
    ///
    /// Has to be called on the main thread once the event loop runs (the
    /// menu is made when the application finished launching); elsewhere,
    /// and a second time, it does nothing. What else ends a program (the
    /// Dock's "Quit", a logout) still goes through `terminate:`.
    pub fn take_over_quit(quit: impl Fn() + Send + Sync + 'static) -> bool {
        if MainThreadMarker::new().is_none()
            || QUIT.set(Box::new(quit)).is_err()
        {
            return false;
        }

        let mut found = false;

        // SAFETY: on the main thread. The menus and their items are
        // checked for nil; the listener answers the selector it is given
        // and lives as long as the process (a menu item does not keep its
        // target alive).
        unsafe {
            let application: *mut AnyObject =
                msg_send![class!(NSApplication), sharedApplication];
            let menu: *mut AnyObject = msg_send![application, mainMenu];

            if menu.is_null() {
                return false;
            }

            let entries: isize = msg_send![menu, numberOfItems];

            for entry in 0..entries {
                let entry: *mut AnyObject = msg_send![menu, itemAtIndex: entry];

                if entry.is_null() {
                    continue;
                }

                let submenu: *mut AnyObject = msg_send![entry, submenu];

                if submenu.is_null() {
                    continue;
                }

                let items: isize = msg_send![submenu, numberOfItems];

                for item in 0..items {
                    let item: *mut AnyObject =
                        msg_send![submenu, itemAtIndex: item];

                    if item.is_null() {
                        continue;
                    }

                    let action: Option<Sel> = msg_send![item, action];

                    if action == Some(sel!(terminate:)) {
                        let _: () = msg_send![item, setTarget: listener()];
                        let _: () = msg_send![
                            item,
                            setAction: sel!(icedQuitRequested:)
                        ];

                        found = true;
                    }
                }
            }
        }

        found
    }

    /// Calls `look` with `AppleInterfaceStyle` of the settings of the
    /// system ("Dark", or nothing for light) whenever macOS says that the
    /// look changed. Has to be called on the main thread; elsewhere, and
    /// a second time, it does nothing.
    pub fn watch_look(look: impl Fn(Option<String>) + Send + Sync + 'static) {
        if MainThreadMarker::new().is_none()
            || LOOK.set(Box::new(look)).is_err()
        {
            return;
        }

        // SAFETY: on the main thread. The listener answers the selector
        // and lives as long as the process (the notification center does
        // not keep an observer alive).
        unsafe {
            let center: *mut AnyObject = msg_send![
                class!(NSDistributedNotificationCenter),
                defaultCenter
            ];

            if center.is_null() {
                return;
            }

            let nobody: *mut AnyObject = std::ptr::null_mut();

            let _: () = msg_send![
                center,
                addObserver: listener(),
                selector: sel!(icedLookChanged:),
                name: string(c"AppleInterfaceThemeChangedNotification"),
                object: nobody
            ];
        }
    }
}
