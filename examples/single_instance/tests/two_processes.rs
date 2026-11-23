//! The built example, started twice: the second process ends at once and
//! the first one tells what it was handed.
//!
//! Needs a display (the first process opens a window); without one the
//! test says so and passes. Each run has a runtime directory of its own,
//! so it neither meets nor disturbs a copy that somebody runs by hand.
#![cfg(target_os = "linux")]

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn the_second_start_hands_its_arguments_to_the_first_and_ends() {
    if std::env::var_os("DISPLAY").is_none()
        && std::env::var_os("WAYLAND_DISPLAY").is_none()
    {
        eprintln!("no display: the two processes are not started");
        return;
    }

    let program = env!("CARGO_BIN_EXE_single_instance");

    let runtime = std::env::temp_dir().join(format!(
        "iced-single-instance-example-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&runtime).unwrap();

    let command = || {
        let mut command = Command::new(program);
        let _ = command
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("ICED_BACKEND", "tiny-skia")
            .env_remove("WAYLAND_DISPLAY");
        command
    };

    let mut first = command()
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    // The first one listens once its socket is there
    let socket =
        runtime.join("com.lumcells.iced.examples.single-instance.sock");
    let deadline = Instant::now() + Duration::from_secs(30);

    while !socket.exists() {
        assert!(Instant::now() < deadline, "the first start never listened");
        assert!(
            first.try_wait().unwrap().is_none(),
            "the first start ended early"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // The second one ends at once, with success
    let started = Instant::now();
    let second = command()
        .args(["--open", "a file.txt"])
        .env("XDG_ACTIVATION_TOKEN", "token-of-the-test")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();

    assert!(second.success());
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the second start did not end at once"
    );

    // And the first one was handed its arguments
    let mut lines = BufReader::new(first.stdout.take().unwrap()).lines();
    let line = lines.next().unwrap().unwrap();
    assert_eq!(line, r#"started again with ["--open", "a file.txt"]"#);

    // It still runs
    assert!(first.try_wait().unwrap().is_none());

    first.kill().unwrap();
    let _ = first.wait();
    let _ = std::fs::remove_dir_all(&runtime);
}
