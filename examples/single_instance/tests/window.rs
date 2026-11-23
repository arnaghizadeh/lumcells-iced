//! The built example on an X server of its own with a real window manager
//! (Xvfb and openbox, started by the test; nothing is shown on, or asked
//! of, the desktop of whoever runs this):
//!
//! - a second start brings the window of the first to the front, also
//!   when it was minimized (`window::present`);
//! - the window is opened again where, and as large as, it was closed
//!   (`window::Settings::remember`).
//!
//! Needs `Xvfb`, `openbox`, `xdotool`, `xprop` and `wmctrl`; where one is
//! missing the test says so and passes.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const ID: &str = "com.lumcells.iced.examples.single-instance";

/// A process that is ended when the test is over, however it ends.
struct Ended(Child);

impl Drop for Ended {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn has(tool: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {tool}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn output(display: &str, tool: &str, arguments: &[&str]) -> String {
    let output = Command::new(tool)
        .args(arguments)
        .env("DISPLAY", display)
        .env_remove("WAYLAND_DISPLAY")
        .stderr(Stdio::null())
        .output()
        .unwrap();

    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn wait_for<T>(what: &str, mut look: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        if let Some(found) = look() {
            return found;
        }

        assert!(Instant::now() < deadline, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The window of the example, as the X server numbers it.
fn window(display: &str) -> String {
    wait_for("the window of the example is shown", || {
        let found = output(
            display,
            "xdotool",
            &["search", "--onlyvisible", "--name", "^Single instance$"],
        );

        found.lines().next().map(str::to_owned)
    })
}

fn is_minimized(display: &str, window: &str) -> bool {
    output(display, "xprop", &["-id", window, "WM_STATE"]).contains("Iconic")
}

fn active(display: &str) -> String {
    output(display, "xdotool", &["getactivewindow"])
}

/// Where the window is and how large: x, y, width, height.
fn geometry(display: &str, window: &str) -> (i32, i32, i32, i32) {
    let shell = output(
        display,
        "xdotool",
        &["getwindowgeometry", "--shell", window],
    );

    let value = |name: &str| {
        shell
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("no {name} in {shell:?}"))
    };

    (value("X"), value("Y"), value("WIDTH"), value("HEIGHT"))
}

struct Desk {
    display: String,
    directory: PathBuf,
    _server: Ended,
    _manager: Ended,
}

impl Desk {
    fn start(name: &str) -> Option<Self> {
        for tool in ["Xvfb", "openbox", "xdotool", "xprop", "wmctrl"] {
            if !has(tool) {
                eprintln!("no {tool} here: the window is not tested");
                return None;
            }
        }

        let directory = std::env::temp_dir().join(format!(
            "iced-single-instance-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("runtime")).unwrap();
        std::fs::create_dir_all(directory.join("home")).unwrap();

        // The server chooses a free display and tells its number
        let mut server = Command::new("Xvfb")
            .args([
                "-displayfd",
                "1",
                "-screen",
                "0",
                "1280x800x24",
                "-nolisten",
                "tcp",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let mut number = String::new();
        let _ = BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut number)
            .unwrap();
        let server = Ended(server);

        let display = format!(":{}", number.trim());
        assert!(display.len() > 1, "Xvfb told no display");

        let manager = Ended(
            Command::new("openbox")
                .arg("--sm-disable")
                .env("DISPLAY", &display)
                .env("HOME", directory.join("home"))
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("XDG_CONFIG_HOME")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );

        // The window manager is there once it says who it is
        wait_for("the window manager runs", || {
            output(&display, "xprop", &["-root", "_NET_SUPPORTING_WM_CHECK"])
                .contains("window id")
                .then_some(())
        });

        Some(Self {
            display,
            directory,
            _server: server,
            _manager: manager,
        })
    }

    fn example(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_single_instance"));
        let _ = command
            .env("DISPLAY", &self.display)
            .env("XDG_RUNTIME_DIR", self.directory.join("runtime"))
            .env("XDG_STATE_HOME", self.directory.join("state"))
            .env("HOME", self.directory.join("home"))
            .env("ICED_BACKEND", "tiny-skia")
            .env("WINIT_X11_SCALE_FACTOR", "1")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("XDG_ACTIVATION_TOKEN")
            .env_remove("DESKTOP_STARTUP_ID")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    fn first(&self) -> Ended {
        let first = Ended(self.example().spawn().unwrap());
        let socket = self.directory.join("runtime").join(format!("{ID}.sock"));

        wait_for("the first start listens", || socket.exists().then_some(()));

        first
    }

    fn remembered(&self) -> PathBuf {
        self.directory
            .join("state")
            .join(ID)
            .join("window-main.conf")
    }
}

impl Drop for Desk {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn a_second_start_brings_a_minimized_window_back_to_the_front() {
    let Some(desk) = Desk::start("present") else {
        return;
    };

    let _first = desk.first();
    let window = window(&desk.display);

    // The person minimizes the window
    let _ = output(&desk.display, "xdotool", &["windowminimize", &window]);
    wait_for("the window is minimized", || {
        is_minimized(&desk.display, &window).then_some(())
    });
    assert_ne!(active(&desk.display), window);

    // ... and starts the program again
    assert!(desk.example().arg("--again").status().unwrap().success());

    // The window is back, and it is the active one
    wait_for("the window is restored and active", || {
        (!is_minimized(&desk.display, &window)
            && active(&desk.display) == window)
            .then_some(())
    });

    // Behind another window (an `xmessage` would do; the root has none
    // here), the same request is what raises it: not tested apart
}

#[test]
fn a_window_is_opened_again_where_and_as_large_as_it_was_closed() {
    let Some(desk) = Desk::start("remember") else {
        return;
    };

    let mut first = desk.first();
    let window_id = window(&desk.display);

    // Nothing is remembered yet: the size of the program, 420 x 260
    let (_, _, width, height) = geometry(&desk.display, &window_id);
    assert_eq!((width, height), (420, 260));

    // The person makes it larger and moves it
    let _ = output(
        &desk.display,
        "xdotool",
        &["windowsize", &window_id, "560", "340"],
    );
    let _ = output(
        &desk.display,
        "xdotool",
        &["windowmove", &window_id, "300", "180"],
    );

    let left = wait_for("the window is moved and resized", || {
        let now = geometry(&desk.display, &window_id);

        (now.2 == 560 && now.3 == 340 && now.0 >= 290).then_some(now)
    });

    // ... and closes it, as its close button does
    std::thread::sleep(Duration::from_millis(300));
    let _ = output(&desk.display, "wmctrl", &["-i", "-c", &window_id]);
    let _ = wait_for("the program ends with its window", || {
        first.0.try_wait().unwrap()
    });

    // Its file says the size, in the place for the state of the program
    let file = read(&desk.remembered());
    assert!(file.contains("width=560\n"), "{file}");
    assert!(file.contains("height=340\n"), "{file}");
    assert!(file.contains("maximized=false\n"), "{file}");
    assert!(file.contains("x="), "{file}");

    // Started again: the same window, at the same place
    let _second = desk.first();
    let again = window(&desk.display);

    let now = wait_for("the window is where it was", || {
        let now = geometry(&desk.display, &again);

        (now.2 == 560).then_some(now)
    });
    assert_eq!(now, left, "{file}");
}
