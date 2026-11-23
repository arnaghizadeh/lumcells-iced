//! Configure the shell that runs a program.
//!
//! These are the settings that the Lumcells fork adds to the ones of
//! [`iced_core::Settings`]. They travel on their own, so that the `Settings`
//! struct of `0.14` stays as it is.

/// The settings of the shell that runs a program.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Settings {
    /// Whether Tab and Shift+Tab move the keyboard focus to the next and the
    /// previous focusable widget when no widget captured the key.
    ///
    /// A key press that moved the focus counts as captured: subscriptions
    /// that listen for ignored key presses do not see it.
    ///
    /// By default, it is enabled.
    pub tab_focus: bool,

    /// The tray icon the program starts with, if any; see
    /// [`tray`](crate::tray).
    ///
    /// By default, there is none.
    pub tray: Option<crate::tray::Settings>,

    /// The identifier under which one instance of the program runs per
    /// user (a reverse domain name, like `com.example.app`): a process
    /// started while another one runs hands it its arguments and ends.
    ///
    /// By default, there is none: every start is a program of its own.
    pub single_instance: Option<String>,

    /// The [`ActivationPolicy`] the program starts with on macOS.
    ///
    /// By default, the one of the platform ([`ActivationPolicy::Regular`]).
    pub activation_policy: Option<ActivationPolicy>,
}

/// How a program shows on macOS. Nothing on other platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ActivationPolicy {
    /// An ordinary program: an icon in the Dock, a menu bar, a place in
    /// the application switcher.
    #[default]
    Regular,
    /// A program without a Dock icon and menu bar, that can still show
    /// windows and be active: one that lives in the menu bar (the tray).
    Accessory,
    /// A program that shows nothing and cannot be made active.
    Prohibited,
}

/// A shell operation.
#[derive(Debug, Clone)]
pub enum Action {
    /// Change the [`ActivationPolicy`] of the running program (macOS).
    SetActivationPolicy(ActivationPolicy),
}

/// Changes the [`ActivationPolicy`] of the running program on macOS: for
/// instance to [`ActivationPolicy::Regular`] while a window is open and back
/// to [`ActivationPolicy::Accessory`] when the last one is closed, so that
/// the program is in the Dock only while it has something to show.
///
/// Does nothing on other platforms.
pub fn set_activation_policy<T>(policy: ActivationPolicy) -> crate::Task<T> {
    crate::task::effect(crate::Action::Shell(Action::SetActivationPolicy(
        policy,
    )))
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tab_focus: true,
            tray: None,
            single_instance: None,
            activation_policy: None,
        }
    }
}
