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

    /// Whether the program is in the Dock of macOS only while it has a
    /// window: [`ActivationPolicy::Regular`] from its first window on,
    /// [`ActivationPolicy::Accessory`] again when the last one is closed.
    /// For a program that lives in the menu bar; start it with
    /// [`ActivationPolicy::Accessory`].
    ///
    /// By default, it is disabled: the policy stays what it was set to.
    pub dock_follows_windows: bool,

    /// Whether the program becomes the active application of macOS when
    /// it starts. A program that starts without a window (at login, with
    /// its menu bar icon only) should not: it would take the keyboard from
    /// what the person is typing in.
    ///
    /// By default, it does, as every program started by a person.
    pub activate_at_start: bool,

    /// Whether the keys of the platform for windows and the program do
    /// what they do everywhere on it, when no widget captured them. On
    /// macOS: Command-W asks the window with the focus to close, as its
    /// red button does; "Quit" in the menu of the application (Command-Q)
    /// asks the program to quit (see `system::quit_requests`) instead of
    /// ending the process inside AppKit. Nothing on other platforms.
    ///
    /// By default, it is enabled.
    pub platform_keys: bool,

    /// Whether the program ends when the person asks it to quit in the way
    /// of the platform (see [`platform_keys`](Self::platform_keys)).
    ///
    /// By default, it does.
    pub exit_on_quit_request: bool,

    /// Whether the shell itself watches the look of the system (light or
    /// dark) where the window library does not tell reliably: on macOS a
    /// window that was given the look of the program's theme is no longer
    /// told of the system's changes, and on Windows the change is told one
    /// message late. The shell listens to the system directly and reports
    /// what `system::theme_changes` reports.
    ///
    /// By default, it is enabled.
    pub watch_system_look: bool,
}

/// What a key of the platform asks for; see
/// [`Settings::platform_keys`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlatformKey {
    /// Close the window with the focus.
    CloseWindow,
    /// Quit the program.
    Quit,
}

impl PlatformKey {
    /// What the given event asks for on a platform, if it is one of its
    /// keys: on macOS Command-W and Command-Q, with no other modifier.
    pub fn of(is_macos: bool, event: &crate::core::Event) -> Option<Self> {
        use crate::core::keyboard::{self, Key, Modifiers};

        let crate::core::Event::Keyboard(keyboard::Event::KeyPressed {
            key,
            modifiers,
            ..
        }) = event
        else {
            return None;
        };

        if !is_macos || *modifiers != Modifiers::LOGO {
            return None;
        }

        match key.as_ref() {
            Key::Character("w" | "W") => Some(Self::CloseWindow),
            Key::Character("q" | "Q") => Some(Self::Quit),
            _ => None,
        }
    }
}

impl ActivationPolicy {
    /// The policy of a program that is in the Dock only while it has a
    /// window; see [`Settings::dock_follows_windows`].
    pub fn for_windows(open: usize) -> Self {
        if open == 0 {
            Self::Accessory
        } else {
            Self::Regular
        }
    }
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
            dock_follows_windows: false,
            activate_at_start: true,
            platform_keys: true,
            exit_on_quit_request: true,
            watch_system_look: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::core::Event;
    use crate::core::keyboard::{self, Key, Modifiers, key::Named};

    fn pressed(key: Key, modifiers: Modifiers) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn command_w_and_command_q_are_keys_of_macos_only() {
        let w = || Key::Character("w".into());
        let q = || Key::Character("q".into());

        assert_eq!(
            PlatformKey::of(true, &pressed(w(), Modifiers::LOGO)),
            Some(PlatformKey::CloseWindow)
        );
        assert_eq!(
            PlatformKey::of(
                true,
                &pressed(Key::Character("W".into()), Modifiers::LOGO)
            ),
            Some(PlatformKey::CloseWindow)
        );
        assert_eq!(
            PlatformKey::of(true, &pressed(q(), Modifiers::LOGO)),
            Some(PlatformKey::Quit)
        );

        // A plain letter is a letter, and with more held it is another key
        assert_eq!(
            PlatformKey::of(true, &pressed(w(), Modifiers::empty())),
            None
        );
        assert_eq!(
            PlatformKey::of(
                true,
                &pressed(w(), Modifiers::LOGO | Modifiers::SHIFT)
            ),
            None
        );
        assert_eq!(
            PlatformKey::of(
                true,
                &pressed(Key::Named(Named::Escape), Modifiers::LOGO)
            ),
            None
        );

        // The Windows key and W is not a program's on the other systems
        assert_eq!(
            PlatformKey::of(false, &pressed(w(), Modifiers::LOGO)),
            None
        );
        assert_eq!(
            PlatformKey::of(false, &pressed(q(), Modifiers::CTRL)),
            None
        );
    }

    #[test]
    fn the_dock_holds_the_program_only_while_it_has_a_window() {
        assert_eq!(
            ActivationPolicy::for_windows(0),
            ActivationPolicy::Accessory
        );
        assert_eq!(ActivationPolicy::for_windows(1), ActivationPolicy::Regular);
        assert_eq!(ActivationPolicy::for_windows(3), ActivationPolicy::Regular);
    }

    #[test]
    fn by_default_a_program_behaves_as_before_and_as_its_platform_expects() {
        let settings = Settings::default();

        assert!(!settings.dock_follows_windows);
        assert!(settings.activate_at_start);
        assert!(settings.platform_keys);
        assert!(settings.exit_on_quit_request);
        assert!(settings.watch_system_look);
    }
}
