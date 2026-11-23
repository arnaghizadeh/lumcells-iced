use std::path::PathBuf;

/// Asks that a window be opened where, and as large as, the window with
/// the same key was when it was last closed, by this program on this
/// computer.
///
/// ```
/// use iced_core::window;
///
/// let settings = window::Settings {
///     // The size and the place of the very first time
///     size: (980.0, 680.0).into(),
///     position: window::Position::Centered,
///     remember: Some(window::Remember::new("main")),
///     ..window::Settings::default()
/// };
/// ```
///
/// The shell keeps a small file per key and writes it while the window is
/// moved and resized (at most once a second), when it loses the focus and
/// when it is closed. When a window with the key is opened again:
///
/// - its size is the remembered one (never larger than the monitor it is
///   put on, never smaller than `min_size`), whatever
///   [`Settings::size`](super::Settings::size) says;
/// - its place is the remembered one if the top of the window would still
///   be on a monitor; otherwise (a second screen was taken away, the
///   resolution changed) [`Position::Centered`](super::Position::Centered)
///   on the first monitor;
/// - it is maximized if it was (and then leaves that state for the size
///   it had before).
///
/// With nothing remembered yet (or a file that cannot be read) the
/// settings are taken as they are.
///
/// ## Platform-specific
///
/// - **Wayland:** a program is not told where its windows are and cannot
///   place them. The size and the maximized state are remembered; the
///   place is the compositor's.
/// - **Web:** nothing is remembered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Remember {
    /// What tells this window from the others of the program: `"main"`,
    /// `"settings"`. Letters, digits, `-`, `_` and `.` are kept in the
    /// name of the file; anything else becomes `_`.
    pub key: String,

    /// The directory of the file (`window-<key>.conf`).
    ///
    /// `None` is the place of the system for the state of a program, in a
    /// directory named after the program (its `Settings::id`, or the
    /// identifier it gave for its single instance, or else the name of
    /// its file):
    ///
    /// - Linux: `$XDG_STATE_HOME/<program>` (`~/.local/state/<program>`);
    /// - macOS: `~/Library/Application Support/<program>`;
    /// - Windows: `%LOCALAPPDATA%\<program>`.
    ///
    /// A program that keeps a directory per profile gives that one, so
    /// that each profile remembers its own windows.
    pub directory: Option<PathBuf>,
}

impl Remember {
    /// Remembers the window under the given key, in the place of the
    /// system for the state of the program.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            directory: None,
        }
    }

    /// Keeps the file in the given directory instead (it is created when
    /// it is not there).
    pub fn directory(mut self, directory: impl Into<PathBuf>) -> Self {
        self.directory = Some(directory.into());
        self
    }

    /// The name of the file: `window-<key>.conf`, with whatever a file
    /// name may not hold on some system replaced.
    pub fn file_name(&self) -> String {
        let key: String = self
            .key
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        format!("window-{key}.conf")
    }
}

impl From<&str> for Remember {
    fn from(key: &str) -> Self {
        Self::new(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_becomes_a_file_name_that_every_system_takes() {
        assert_eq!(Remember::new("main").file_name(), "window-main.conf");
        assert_eq!(
            Remember::new("../run: 7/log").file_name(),
            "window-.._run__7_log.conf"
        );
        assert_eq!(
            Remember::from("settings")
                .directory("/tmp/profile")
                .directory,
            Some(PathBuf::from("/tmp/profile"))
        );
    }
}
