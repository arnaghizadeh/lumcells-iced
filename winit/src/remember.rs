//! Open a window where, and as large as, it was when it was last closed
//! ([`window::Remember`]).
//!
//! The rules are plain functions over plain values ([`Geometry`],
//! [`Monitor`], [`restore`], [`Tracker`]); the shell feeds them with what
//! `winit` tells and keeps one small file per remembered window.
use crate::core::Size;
use crate::core::window;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

/// Where a window is and how large: what is kept in its file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Geometry {
    /// The top left corner of the window with its frame, in physical
    /// pixels of the desktop. `None` where a program is not told
    /// (Wayland).
    pub position: Option<(i32, i32)>,
    /// The size of what the program draws, in the logical pixels of
    /// [`window::Settings::size`]; while the window is maximized, the
    /// size it had before.
    pub size: Size,
    /// Whether the window is maximized.
    pub maximized: bool,
}

impl Geometry {
    /// Reads what [`Geometry::write`] wrote. Lines it does not know are
    /// skipped (a later version may write more); a size that is missing
    /// or not a size makes the whole file count as nothing.
    pub fn parse(text: &str) -> Option<Self> {
        let mut values = HashMap::new();

        for line in text.lines() {
            if let Some((name, value)) = line.split_once('=') {
                let _ = values.insert(name.trim(), value.trim());
            }
        }

        let number = |name: &str| values.get(name)?.parse::<f32>().ok();
        let whole = |name: &str| values.get(name)?.parse::<i32>().ok();

        let width = number("width")?;
        let height = number("height")?;

        if !(width.is_finite() && height.is_finite())
            || width < 1.0
            || height < 1.0
        {
            return None;
        }

        Some(Self {
            position: whole("x").zip(whole("y")),
            size: Size::new(width, height),
            maximized: values.get("maximized") == Some(&"true"),
        })
    }

    /// The text of the file: a line per value.
    pub fn write(&self) -> String {
        let mut text = String::from(
            "# Where this window was when it was last used (written by iced)\n",
        );

        if let Some((x, y)) = self.position {
            text.push_str(&format!("x={x}\ny={y}\n"));
        }

        text.push_str(&format!(
            "width={}\nheight={}\nmaximized={}\n",
            self.size.width, self.size.height, self.maximized
        ));

        text
    }
}

/// A monitor of the desktop: where it begins and its size in physical
/// pixels, and its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    /// Where it begins.
    pub position: (i32, i32),
    /// Its size.
    pub size: (u32, u32),
    /// Its scale factor.
    pub scale_factor: f64,
}

/// How a remembered window is opened.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Where its top left corner goes, in physical pixels; `None` when
    /// the place is not known or no longer on a monitor.
    pub position: Option<(i32, i32)>,
    /// Whether it has to be put in the middle of a monitor instead: a
    /// place was remembered and it is no longer on any.
    pub centered: bool,
    /// Its size in the logical pixels of [`window::Settings::size`].
    pub size: Size,
    /// Whether it is maximized.
    pub maximized: bool,
}

/// How much of the top of a window, in logical pixels, has to be on a
/// monitor for the window to be opened where it was: enough of a title
/// bar to take hold of.
const HOLD: (f64, f64) = (96.0, 32.0);

/// How a window that was left with the given [`Geometry`] is opened on
/// the monitors that are there now (the first is the one a window is
/// centered on when its place is gone).
///
/// - The place is kept if the top edge of the window is on a monitor
///   (not above it, and not in its last 32 logical pixels) with at least
///   96 logical pixels of its width (or all of it) inside that monitor.
///   A second screen that was taken away, a smaller resolution, another
///   arrangement: the window is centered instead.
/// - The size is kept in logical pixels, so a window keeps its apparent
///   size when the scale of the screen changes, and is made no larger
///   than the monitor it is opened on, and no smaller than `min_size`.
///
/// `scale_factor` is the one of the program (`application(..)
/// .scale_factor(..)`), which multiplies the one of the monitor.
pub fn restore(
    remembered: Geometry,
    monitors: &[Monitor],
    min_size: Option<Size>,
    scale_factor: f32,
) -> Placement {
    let program_scale = if scale_factor > 0.0 {
        f64::from(scale_factor)
    } else {
        1.0
    };

    let scale_of = |monitor: &Monitor| {
        let scale = if monitor.scale_factor > 0.0 {
            monitor.scale_factor
        } else {
            1.0
        };

        scale * program_scale
    };

    let home = remembered.position.and_then(|(x, y)| {
        monitors
            .iter()
            .filter_map(|monitor| {
                let scale = scale_of(monitor);
                let width = f64::from(remembered.size.width) * scale;

                let left = f64::from(monitor.position.0);
                let right = left + f64::from(monitor.size.0);
                let top = f64::from(monitor.position.1);
                let bottom = top + f64::from(monitor.size.1);

                let (x, y) = (f64::from(x), f64::from(y));

                let inside = (x + width).min(right) - x.max(left);
                let needed = (HOLD.0 * scale).min(width);

                // A frame may begin a few pixels above the screen
                let on_top_edge =
                    y >= top - 16.0 * scale && y < bottom - HOLD.1 * scale;

                (on_top_edge && inside >= needed).then_some((monitor, inside))
            })
            .max_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(monitor, _)| monitor)
    });

    let monitor = home.or(monitors.first());

    let mut size = remembered.size;

    if let Some(monitor) = monitor {
        let scale = scale_of(monitor);

        size.width = size.width.min((f64::from(monitor.size.0) / scale) as f32);
        size.height =
            size.height.min((f64::from(monitor.size.1) / scale) as f32);
    }

    if let Some(min_size) = min_size {
        size.width = size.width.max(min_size.width);
        size.height = size.height.max(min_size.height);
    }

    Placement {
        position: home.and(remembered.position),
        centered: remembered.position.is_some() && home.is_none(),
        size,
        maximized: remembered.maximized,
    }
}

/// What the desktop tells about a window at some moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seen {
    /// It is neither maximized, minimized nor fullscreen: this is its
    /// own place and size.
    Normal {
        /// Its place, where a program is told.
        position: Option<(i32, i32)>,
        /// Its size, in the logical pixels of [`window::Settings::size`].
        size: Size,
    },
    /// It is maximized.
    Maximized,
    /// It is minimized or fullscreen: nothing to remember.
    Away,
}

/// Follows a window and says when its file has to be written.
#[derive(Debug, Clone)]
pub struct Tracker {
    geometry: Option<Geometry>,
    /// The place and size the window had before it last began to change.
    settled: Option<(Option<(i32, i32)>, Size)>,
    changed_at: Option<Instant>,
    written: Option<Geometry>,
    written_at: Option<Instant>,
}

impl Tracker {
    /// The file is written at most this often while a window changes.
    pub const PACE: Duration = Duration::from_secs(1);

    /// Some desktops tell the size of a maximized window before they say
    /// that it is maximized: a change this shortly before is taken as
    /// part of the maximizing, not as the size of the window itself.
    pub const SETTLE: Duration = Duration::from_millis(500);

    /// A [`Tracker`] of a window that was opened with the given
    /// [`Geometry`] from its file, if it had one.
    pub fn new(remembered: Option<Geometry>) -> Self {
        Self {
            geometry: remembered,
            settled: remembered
                .map(|geometry| (geometry.position, geometry.size)),
            changed_at: None,
            written: remembered,
            written_at: None,
        }
    }

    /// What would be written now.
    pub fn geometry(&self) -> Option<Geometry> {
        self.geometry
    }

    /// Takes in what the desktop tells about the window at `now`, and
    /// returns what has to be written to its file at once, if anything.
    pub fn see(&mut self, seen: Seen, now: Instant) -> Option<Geometry> {
        match seen {
            Seen::Away => return None,
            Seen::Normal { position, size } => {
                if size.width < 1.0 || size.height < 1.0 {
                    return None;
                }

                let before = self
                    .geometry
                    .map(|geometry| (geometry.position, geometry.size));

                if before != Some((position, size)) {
                    let is_new_change = self
                        .changed_at
                        .is_none_or(|at| now.duration_since(at) > Self::SETTLE);

                    if is_new_change {
                        self.settled = before;
                    }

                    self.changed_at = Some(now);
                }

                self.geometry = Some(Geometry {
                    position,
                    size,
                    maximized: false,
                });
            }
            Seen::Maximized => {
                let in_the_same_breath = self
                    .changed_at
                    .is_some_and(|at| now.duration_since(at) <= Self::SETTLE);

                let own = if in_the_same_breath {
                    self.settled
                } else {
                    None
                }
                .or(self
                    .geometry
                    .map(|geometry| (geometry.position, geometry.size)));

                // A window that was never seen out of the maximized
                // state has no size of its own to remember
                let (position, size) = own?;

                self.changed_at = None;
                self.geometry = Some(Geometry {
                    position,
                    size,
                    maximized: true,
                });
            }
        }

        if self.geometry == self.written {
            return None;
        }

        let is_due = self
            .written_at
            .is_none_or(|at| now.duration_since(at) >= Self::PACE);

        if is_due { self.flush(now) } else { None }
    }

    /// Returns what has to be written now whatever the pace: the window
    /// lost the focus, is being closed, or the program ends.
    pub fn flush(&mut self, now: Instant) -> Option<Geometry> {
        if self.geometry == self.written {
            return None;
        }

        self.written = self.geometry;
        self.written_at = Some(now);

        self.geometry
    }
}

/// The directory for the state of the program with the given name, on
/// the system named by `os` (`std::env::consts::OS`), with the variables
/// of the environment read through `variable`.
pub fn state_directory(
    os: &str,
    program: &str,
    variable: impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    let variable =
        |name: &str| variable(name).filter(|value| !value.is_empty());

    let base = match os {
        "windows" => variable("LOCALAPPDATA")
            .or_else(|| variable("APPDATA"))
            .map(PathBuf::from)?,
        "macos" => PathBuf::from(variable("HOME")?)
            .join("Library")
            .join("Application Support"),
        _ => match variable("XDG_STATE_HOME") {
            // The specification: a path that is not absolute is ignored
            Some(state) if Path::new(&state).is_absolute() => {
                PathBuf::from(state)
            }
            _ => PathBuf::from(variable("HOME")?)
                .join(".local")
                .join("state"),
        },
    };

    Some(base.join(program))
}

/// The name of a program as a directory name.
fn directory_name(program: &str) -> String {
    let name: String = program
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();

    let name = name.trim_matches(|c| c == '.' || c == ' ');

    if name.is_empty() {
        "iced".to_owned()
    } else {
        name.to_owned()
    }
}

static PROGRAM: OnceLock<String> = OnceLock::new();

/// Tells this module what the program is called: the first of the given
/// names that is there (its `Settings::id`, the identifier of its single
/// instance), or else the name of its file.
pub(crate) fn name_program<'a>(
    names: impl IntoIterator<Item = Option<&'a str>>,
) {
    let name = names
        .into_iter()
        .flatten()
        .find(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            std::env::current_exe().ok().and_then(|path| {
                path.file_stem()
                    .map(|name| name.to_string_lossy().into_owned())
            })
        })
        .unwrap_or_else(|| "iced".to_owned());

    let _ = PROGRAM.set(directory_name(&name));
}

/// The file of a remembered window.
pub fn file(remember: &window::Remember) -> Option<PathBuf> {
    let directory = match &remember.directory {
        Some(directory) => directory.clone(),
        None => state_directory(
            std::env::consts::OS,
            PROGRAM.get().map(String::as_str).unwrap_or("iced"),
            |name| std::env::var(name).ok(),
        )?,
    };

    Some(directory.join(remember.file_name()))
}

/// Reads the file of a window; `None` when it is not there or is not
/// what [`save`] writes.
pub fn load(path: &Path) -> Option<Geometry> {
    Geometry::parse(&std::fs::read_to_string(path).ok()?)
}

/// Writes the file of a window (and makes its directory): to a file
/// beside it first, which then takes its name, so that a file is never
/// half written.
pub fn save(path: &Path, geometry: &Geometry) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }

    let mut beside = path.as_os_str().to_owned();
    beside.push(".new");
    let beside = PathBuf::from(beside);

    std::fs::write(&beside, geometry.write())?;
    std::fs::rename(&beside, path)
}

/// A remembered window that is about to be opened.
pub(crate) struct Opening {
    path: PathBuf,
    remembered: Option<Geometry>,
    scale_factor: f32,
    /// Where the window has to be put, in physical pixels.
    pub position: Option<(i32, i32)>,
}

/// Changes the settings of a window that is to be remembered into what
/// was remembered, before the window is made. `None` for a window that
/// is not remembered (or cannot be: no directory).
pub(crate) fn open(
    settings: &mut window::Settings,
    scale_factor: f32,
    monitors: &[Monitor],
) -> Option<Opening> {
    let remember = settings.remember.as_ref()?;

    // Nowhere to keep a file
    if cfg!(target_arch = "wasm32") {
        return None;
    }

    let Some(path) = file(remember) else {
        log::warn!(
            "the window `{}` is not remembered: no directory for the state \
            of the program",
            remember.key
        );
        return None;
    };

    let remembered = load(&path);
    let mut position = None;

    if let Some(remembered) = remembered {
        let placement =
            restore(remembered, monitors, settings.min_size, scale_factor);

        settings.size = placement.size;
        settings.maximized = placement.maximized;

        if let Some(max_size) = settings.max_size {
            settings.size.width = settings.size.width.min(max_size.width);
            settings.size.height = settings.size.height.min(max_size.height);
        }

        if placement.position.is_some() {
            // Given to the window in physical pixels, as it was read
            settings.position = window::Position::Default;
            position = placement.position;
        } else if placement.centered {
            settings.position = window::Position::Centered;
        }

        log::info!(
            "window `{}` opened as remembered in {}: {placement:?}",
            remember.key,
            path.display()
        );
    }

    Some(Opening {
        path,
        remembered,
        scale_factor,
        position,
    })
}

struct Entry {
    window: Weak<winit::window::Window>,
    path: PathBuf,
    scale_factor: f32,
    tracker: Tracker,
}

static WINDOWS: Mutex<Option<HashMap<winit::window::WindowId, Entry>>> =
    Mutex::new(None);

fn windows() -> std::sync::MutexGuard<
    'static,
    Option<HashMap<winit::window::WindowId, Entry>>,
> {
    WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Starts to follow a window that was made from an [`Opening`].
pub(crate) fn track(window: &Arc<winit::window::Window>, opening: Opening) {
    let _ = windows().get_or_insert_with(HashMap::new).insert(
        window.id(),
        Entry {
            window: Arc::downgrade(window),
            path: opening.path,
            scale_factor: opening.scale_factor,
            tracker: Tracker::new(opening.remembered),
        },
    );
}

fn write(path: &Path, geometry: Option<Geometry>) {
    if let Some(geometry) = geometry
        && let Err(error) = save(path, &geometry)
    {
        log::warn!(
            "the place of a window could not be written to {}: {error}",
            path.display()
        );
    }
}

fn seen(window: &winit::window::Window, scale_factor: f32) -> Seen {
    if window.is_minimized() == Some(true) || window.fullscreen().is_some() {
        return Seen::Away;
    }

    if window.is_maximized() {
        return Seen::Maximized;
    }

    let scale = window.scale_factor()
        * f64::from(if scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        });
    let size = window.inner_size();

    Seen::Normal {
        position: window
            .outer_position()
            .ok()
            .map(|position| (position.x, position.y)),
        size: Size::new(
            (f64::from(size.width) / scale) as f32,
            (f64::from(size.height) / scale) as f32,
        ),
    }
}

/// Follows the events of the windows that are remembered.
pub(crate) fn window_event(
    id: winit::window::WindowId,
    event: &winit::event::WindowEvent,
) {
    use winit::event::WindowEvent;

    let is_change = matches!(
        event,
        WindowEvent::Moved(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
    );

    let is_pause = matches!(
        event,
        WindowEvent::Focused(false)
            | WindowEvent::CloseRequested
            | WindowEvent::Destroyed
    );

    if !is_change && !is_pause {
        return;
    }

    let mut windows = windows();

    let Some(entry) = windows.as_mut().and_then(|windows| windows.get_mut(&id))
    else {
        return;
    };

    let now = Instant::now();
    let mut due = None;

    if let Some(window) = entry.window.upgrade() {
        due = entry.tracker.see(seen(&window, entry.scale_factor), now);
    }

    if is_pause {
        due = entry.tracker.flush(now).or(due);
    }

    write(&entry.path, due);

    if matches!(event, WindowEvent::Destroyed) {
        let _ = windows.as_mut().and_then(|windows| windows.remove(&id));
    }
}

/// A window is closed by the program: what is known of it is written and
/// it is no longer followed.
pub(crate) fn close(window: &winit::window::Window) {
    let entry = windows()
        .as_mut()
        .and_then(|windows| windows.remove(&window.id()));

    if let Some(mut entry) = entry {
        let now = Instant::now();

        let _ = entry.tracker.see(seen(window, entry.scale_factor), now);
        write(&entry.path, entry.tracker.flush(now));
    }
}

/// The program ends: everything that waits is written.
pub(crate) fn flush_all() {
    if let Some(windows) = windows().as_mut() {
        if windows.is_empty() {
            return;
        }

        let now = Instant::now();

        for entry in windows.values_mut() {
            write(&entry.path, entry.tracker.flush(now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_HD: Monitor = Monitor {
        position: (0, 0),
        size: (1920, 1080),
        scale_factor: 1.0,
    };

    /// A second screen on the right of the first.
    const SECOND: Monitor = Monitor {
        position: (1920, 0),
        size: (2560, 1440),
        scale_factor: 1.0,
    };

    fn at(x: i32, y: i32, width: f32, height: f32) -> Geometry {
        Geometry {
            position: Some((x, y)),
            size: Size::new(width, height),
            maximized: false,
        }
    }

    #[test]
    fn a_file_is_read_back_as_it_was_written() {
        let directory = std::env::temp_dir().join(format!(
            "iced-remember-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);

        let remember = window::Remember::new("main: 1")
            .directory(directory.join("profile"));
        let path = file(&remember).unwrap();
        assert_eq!(path, directory.join("profile").join("window-main__1.conf"));

        // Nothing yet
        assert_eq!(load(&path), None);

        let geometry = Geometry {
            position: Some((-1200, 80)),
            size: Size::new(980.5, 680.0),
            maximized: true,
        };

        // The directory is made
        save(&path, &geometry).unwrap();
        assert_eq!(load(&path), Some(geometry));

        // Written again, over the first; nothing is left beside it
        let moved = at(40, 60, 800.0, 600.0);
        save(&path, &moved).unwrap();
        assert_eq!(load(&path), Some(moved));
        assert_eq!(
            std::fs::read_dir(directory.join("profile"))
                .unwrap()
                .count(),
            1
        );

        // Where a program is not told its place (Wayland), there is none
        let placeless = Geometry {
            position: None,
            ..moved
        };
        save(&path, &placeless).unwrap();
        assert_eq!(load(&path), Some(placeless));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("x="));

        // What is not such a file counts as nothing
        std::fs::write(&path, "width=abc\nheight=600\n").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, "x=1\ny=2\nwidth=0\nheight=600\n").unwrap();
        assert_eq!(load(&path), None);
        std::fs::write(&path, "\u{0}\u{1}garbage").unwrap();
        assert_eq!(load(&path), None);

        // A line of a later version is skipped; half a place is no place
        std::fs::write(&path, "monitor=DP-1\nx=5\nwidth=640\nheight=480\n")
            .unwrap();
        assert_eq!(
            load(&path),
            Some(Geometry {
                position: None,
                size: Size::new(640.0, 480.0),
                maximized: false
            })
        );

        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_window_is_opened_where_it_was_left() {
        let placement =
            restore(at(300, 200, 980.0, 680.0), &[FULL_HD, SECOND], None, 1.0);

        assert_eq!(
            placement,
            Placement {
                position: Some((300, 200)),
                centered: false,
                size: Size::new(980.0, 680.0),
                maximized: false,
            }
        );

        // On the second screen too, and partly out of it at the right
        // and the bottom, as the person left it
        let placement = restore(
            at(4000, 1000, 980.0, 680.0),
            &[FULL_HD, SECOND],
            None,
            1.0,
        );
        assert_eq!(placement.position, Some((4000, 1000)));
        assert!(!placement.centered);

        // Maximized: it is, with its own place and size kept for later
        let placement = restore(
            Geometry {
                maximized: true,
                ..at(300, 200, 980.0, 680.0)
            },
            &[FULL_HD],
            None,
            1.0,
        );
        assert!(placement.maximized);
        assert_eq!(placement.position, Some((300, 200)));
        assert_eq!(placement.size, Size::new(980.0, 680.0));
    }

    #[test]
    fn a_window_whose_monitor_went_away_is_centered_and_made_to_fit() {
        // It was on the second screen, which is no longer there
        let placement =
            restore(at(2500, 300, 2200.0, 1300.0), &[FULL_HD], None, 1.0);

        assert_eq!(placement.position, None);
        assert!(placement.centered);
        assert_eq!(placement.size, Size::new(1920.0, 1080.0));

        // Left of everything, above everything, below everything
        for (x, y) in [(-3000, 100), (100, -500), (100, 1070), (1900, 100)] {
            let placement =
                restore(at(x, y, 800.0, 600.0), &[FULL_HD], None, 1.0);

            assert_eq!(placement.position, None, "{x}, {y}");
            assert!(placement.centered, "{x}, {y}");
            assert_eq!(placement.size, Size::new(800.0, 600.0));
        }

        // A frame that begins a little above the screen is still there,
        // and a narrow window that is inside as a whole
        assert_eq!(
            restore(at(100, -8, 800.0, 600.0), &[FULL_HD], None, 1.0).position,
            Some((100, -8))
        );
        assert_eq!(
            restore(at(1850, 100, 60.0, 600.0), &[FULL_HD], None, 1.0).position,
            Some((1850, 100))
        );

        // No monitor is told at all: as remembered, nothing to fit to
        let placement = restore(at(10, 10, 800.0, 600.0), &[], None, 1.0);
        assert_eq!(placement.position, None);
        assert!(placement.centered);
        assert_eq!(placement.size, Size::new(800.0, 600.0));

        // A place that was never known (Wayland) is not a place that
        // went away: the settings of the program decide
        let placement = restore(
            Geometry {
                position: None,
                ..at(0, 0, 800.0, 600.0)
            },
            &[FULL_HD],
            None,
            1.0,
        );
        assert_eq!(placement.position, None);
        assert!(!placement.centered);
    }

    #[test]
    fn a_window_keeps_its_apparent_size_when_the_scale_changes() {
        // Left at 980 x 680 on a screen at 100 %; the same screen is now
        // at 150 %: 1280 x 720 logical pixels
        let scaled = Monitor {
            scale_factor: 1.5,
            ..FULL_HD
        };

        let placement =
            restore(at(100, 100, 980.0, 680.0), &[scaled], None, 1.0);
        assert_eq!(placement.position, Some((100, 100)));
        assert_eq!(placement.size, Size::new(980.0, 680.0));

        // At 200 % (960 x 540) it no longer fits: as large as the screen,
        // and never smaller than the least the program allows
        let dense = Monitor {
            scale_factor: 2.0,
            ..FULL_HD
        };

        let placement =
            restore(at(100, 100, 980.0, 680.0), &[dense], None, 1.0);
        assert_eq!(placement.size, Size::new(960.0, 540.0));

        let placement = restore(
            at(100, 100, 980.0, 680.0),
            &[dense],
            Some(Size::new(820.0, 560.0)),
            1.0,
        );
        assert_eq!(placement.size, Size::new(960.0, 560.0));

        // The scale of the program multiplies the one of the monitor
        let placement =
            restore(at(100, 100, 980.0, 680.0), &[FULL_HD], None, 2.0);
        assert_eq!(placement.size, Size::new(960.0, 540.0));

        // 1366 x 768 at 125 % (1092.8 x 614.4): the height is cut
        let laptop = Monitor {
            position: (0, 0),
            size: (1366, 768),
            scale_factor: 1.25,
        };
        let placement = restore(at(20, 20, 980.0, 680.0), &[laptop], None, 1.0);
        assert_eq!(placement.size.width, 980.0);
        assert!((placement.size.height - 614.4).abs() < 0.01);

        // A monitor that tells no scale counts as one
        let silent = Monitor {
            scale_factor: 0.0,
            ..FULL_HD
        };
        assert_eq!(
            restore(at(0, 0, 4000.0, 600.0), &[silent], None, 0.0).size,
            Size::new(1920.0, 600.0)
        );
    }

    #[test]
    fn the_file_is_written_at_a_pace_and_at_once_when_the_window_pauses() {
        let start = Instant::now();
        let ms = |ms: u64| start + Duration::from_millis(ms);
        let normal = |x: i32, width: f32| Seen::Normal {
            position: Some((x, 50)),
            size: Size::new(width, 600.0),
        };

        let mut tracker = Tracker::new(None);

        // The first thing seen is written
        assert_eq!(
            tracker.see(normal(10, 800.0), ms(0)),
            Some(at(10, 50, 800.0, 600.0))
        );

        // The same again: nothing
        assert_eq!(tracker.see(normal(10, 800.0), ms(10)), None);

        // A drag: many changes within a second, none written...
        for step in 1..50 {
            assert_eq!(
                tracker.see(normal(10 + step, 800.0), ms(10 * step as u64)),
                None
            );
        }

        // ... until the second is over
        assert_eq!(
            tracker.see(normal(100, 800.0), ms(1000)),
            Some(at(100, 50, 800.0, 600.0))
        );

        // A change, then the window loses the focus: written at once
        assert_eq!(tracker.see(normal(120, 800.0), ms(1100)), None);
        assert_eq!(tracker.flush(ms(1200)), Some(at(120, 50, 800.0, 600.0)));
        assert_eq!(tracker.flush(ms(1300)), None);

        // Minimized, or fullscreen: not a place to remember
        assert_eq!(tracker.see(Seen::Away, ms(5000)), None);
        assert_eq!(tracker.geometry(), Some(at(120, 50, 800.0, 600.0)));

        // A window opened from its file writes nothing until it changes
        let mut tracker = Tracker::new(Some(at(5, 5, 640.0, 480.0)));
        assert_eq!(
            tracker.see(
                Seen::Normal {
                    position: Some((5, 5)),
                    size: Size::new(640.0, 480.0)
                },
                ms(0)
            ),
            None
        );
        assert_eq!(tracker.flush(ms(1)), None);
    }

    #[test]
    fn a_maximized_window_keeps_the_size_it_had_before() {
        let start = Instant::now();
        let ms = |ms: u64| start + Duration::from_millis(ms);

        let mut tracker = Tracker::new(None);
        let own = Seen::Normal {
            position: Some((100, 80)),
            size: Size::new(980.0, 680.0),
        };
        let _ = tracker.see(own, ms(0));

        // Maximized ten seconds later. This desktop tells the new size
        // and place first and only then that the window is maximized
        let _ = tracker.see(
            Seen::Normal {
                position: Some((0, 0)),
                size: Size::new(1920.0, 1050.0),
            },
            ms(10_000),
        );
        let _ = tracker.see(Seen::Maximized, ms(10_040));

        assert_eq!(
            tracker.geometry(),
            Some(Geometry {
                maximized: true,
                ..at(100, 80, 980.0, 680.0)
            })
        );
        assert_eq!(tracker.flush(ms(10_050)), tracker.geometry());

        // Restored: its own size again, no longer maximized
        let _ = tracker.see(own, ms(20_000));
        assert_eq!(tracker.geometry(), Some(at(100, 80, 980.0, 680.0)));

        // Resized by hand, and maximized long after: the new size is
        // its own
        let _ = tracker.see(
            Seen::Normal {
                position: Some((100, 80)),
                size: Size::new(1200.0, 700.0),
            },
            ms(30_000),
        );
        let _ = tracker.see(Seen::Maximized, ms(40_000));
        assert_eq!(
            tracker.geometry(),
            Some(Geometry {
                maximized: true,
                ..at(100, 80, 1200.0, 700.0)
            })
        );

        // A window that is opened maximized and never seen otherwise has
        // nothing of its own to remember
        let mut tracker = Tracker::new(None);
        assert_eq!(tracker.see(Seen::Maximized, ms(0)), None);
        assert_eq!(tracker.geometry(), None);
    }

    #[test]
    fn the_state_of_a_program_has_a_place_on_every_system() {
        let none = |_: &str| None;

        let linux = |name: &str| match name {
            "HOME" => Some("/home/u".to_owned()),
            _ => None,
        };
        assert_eq!(
            state_directory("linux", "com.example.app", linux),
            Some(PathBuf::from("/home/u/.local/state/com.example.app"))
        );

        let xdg = |name: &str| match name {
            "HOME" => Some("/home/u".to_owned()),
            "XDG_STATE_HOME" => Some("/var/state".to_owned()),
            _ => None,
        };
        assert_eq!(
            state_directory("linux", "app", xdg),
            Some(PathBuf::from("/var/state/app"))
        );

        // Not absolute, or empty: ignored, as the specification says
        let relative = |name: &str| match name {
            "HOME" => Some("/home/u".to_owned()),
            "XDG_STATE_HOME" => Some("state".to_owned()),
            _ => None,
        };
        assert_eq!(
            state_directory("freebsd", "app", relative),
            Some(PathBuf::from("/home/u/.local/state/app"))
        );

        let mac = |name: &str| (name == "HOME").then(|| "/Users/u".to_owned());
        assert_eq!(
            state_directory("macos", "Example", mac),
            Some(PathBuf::from(
                "/Users/u/Library/Application Support/Example"
            ))
        );

        let windows = |name: &str| {
            (name == "LOCALAPPDATA")
                .then(|| r"C:\Users\u\AppData\Local".to_owned())
        };
        assert_eq!(
            state_directory("windows", "Example", windows),
            Some(PathBuf::from(r"C:\Users\u\AppData\Local").join("Example"))
        );

        // Nowhere to write: the window is simply not remembered
        assert_eq!(state_directory("linux", "app", none), None);
        assert_eq!(state_directory("windows", "app", none), None);

        assert_eq!(directory_name("com.example.app"), "com.example.app");
        assert_eq!(directory_name("../../etc"), "_.._etc");
        assert_eq!(directory_name(".."), "iced");
        assert_eq!(directory_name("Lumcells Drive"), "Lumcells Drive");
    }

    #[test]
    fn the_settings_of_a_window_become_what_was_remembered() {
        let directory = std::env::temp_dir().join(format!(
            "iced-remember-open-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);

        let settings = || window::Settings {
            size: Size::new(980.0, 680.0),
            min_size: Some(Size::new(820.0, 560.0)),
            max_size: Some(Size::new(1600.0, 1000.0)),
            position: window::Position::Centered,
            remember: Some(window::Remember::new("main").directory(&directory)),
            ..window::Settings::default()
        };

        // The first time: as the program says
        let mut first = settings();
        let opening = open(&mut first, 1.0, &[FULL_HD]).unwrap();
        assert_eq!(opening.position, None);
        assert_eq!(first.size, Size::new(980.0, 680.0));
        assert!(matches!(first.position, window::Position::Centered));

        // The person moves and widens it; it is closed
        save(&opening.path, &at(300, 120, 1700.0, 700.0)).unwrap();

        // The second time: where it was, no wider than the program allows
        let mut second = settings();
        let opening = open(&mut second, 1.0, &[FULL_HD]).unwrap();
        assert_eq!(opening.position, Some((300, 120)));
        assert_eq!(second.size, Size::new(1600.0, 700.0));
        assert!(matches!(second.position, window::Position::Default));
        assert!(!second.maximized);

        // Its screen is gone: in the middle of the one that is there
        save(&opening.path, &at(2500, 120, 1000.0, 700.0)).unwrap();
        let mut third = settings();
        let opening = open(&mut third, 1.0, &[FULL_HD]).unwrap();
        assert_eq!(opening.position, None);
        assert!(matches!(third.position, window::Position::Centered));
        assert_eq!(third.size, Size::new(1000.0, 700.0));

        // A window that is not remembered is left alone
        let mut plain = window::Settings::default();
        assert!(open(&mut plain, 1.0, &[FULL_HD]).is_none());

        std::fs::remove_dir_all(&directory).unwrap();
    }
}
