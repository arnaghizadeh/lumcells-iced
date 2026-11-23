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
