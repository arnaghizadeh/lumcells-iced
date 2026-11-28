use crate::time::Instant;
use crate::{Padding, Point, Size};

use std::path::PathBuf;

/// A window-related event.
#[derive(PartialEq, Clone, Debug)]
pub enum Event {
    /// A window was opened.
    Opened {
        /// The position of the opened window. This is relative to the top-left corner of the desktop
        /// the window is on, including virtual desktops. Refers to window's "outer" position,
        /// or the window area, in logical pixels.
        ///
        /// **Note**: Not available in Wayland.
        position: Option<Point>,
        /// The size of the created window. This is its "inner" size, or the size of the
        /// client area, in logical pixels.
        size: Size,
    },

    /// A window was closed.
    Closed,

    /// A window was moved.
    Moved(Point),

    /// A window was resized.
    Resized(Size),

    /// A window changed its scale factor.
    Rescaled(f32),

    /// The part of a window the system keeps for itself changed: how far
    /// the status bar, the sensor housing, the rounded corners and the home
    /// indicator reach in from each edge, in logical pixels. What is laid
    /// out inside this [`Padding`] is covered by nothing and can be touched.
    ///
    /// ## Platform-specific
    ///
    /// - **iOS:** Produced after [`Opened`](Self::Opened) and whenever the
    ///   insets change (a rotation). The window itself is the whole screen.
    /// - **Other platforms:** Not produced.
    SafeAreaChanged(Padding),

    /// A window redraw was requested.
    ///
    /// The [`Instant`] contains the current time.
    RedrawRequested(Instant),

    /// The user has requested for the window to close.
    CloseRequested,

    /// A window was focused.
    Focused,

    /// A window was unfocused.
    Unfocused,

    /// A file is being hovered over the window.
    ///
    /// When the user hovers multiple files at once, this event will be emitted
    /// for each file separately.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland:** Not implemented.
    FileHovered(PathBuf),

    /// A file has been dropped into the window.
    ///
    /// When the user drops multiple files at once, this event will be emitted
    /// for each file separately.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland:** Not implemented.
    FileDropped(PathBuf),

    /// A file was hovered, but has exited the window.
    ///
    /// There will be a single `FilesHoveredLeft` event triggered even if
    /// multiple files were hovered.
    ///
    /// ## Platform-specific
    ///
    /// - **Wayland:** Not implemented.
    FilesHoveredLeft,
}
