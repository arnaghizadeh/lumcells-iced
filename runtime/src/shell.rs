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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tab_focus: true,
            tray: None,
        }
    }
}
