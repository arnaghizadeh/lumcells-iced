//! Android: the activity the program runs in.
//!
//! An Android program made with `winit` is a library that the activity
//! (a `GameActivity` or a `NativeActivity`) loads; the system calls its
//! `android_main` with an [`AndroidApp`]. Hand it over with [`set_app`]
//! before the program is run:
//!
//! ```ignore
//! #[unsafe(no_mangle)]
//! fn android_main(app: iced::android::AndroidApp) {
//!     iced::android::set_app(app);
//!     iced::application(App::new, App::update, App::view).run().unwrap();
//! }
//! ```
//!
//! What a person types on the system's keyboard does not reach `winit`: the
//! activity's text input takes it. The program's shell (Kotlin or Java)
//! catches it in its `InputConnection` and hands it over with [`text_input`].
use std::sync::{Mutex, OnceLock};

pub use winit::platform::android::activity;
pub use winit::platform::android::activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();

/// Says which activity the program runs in. Called once, in
/// `android_main`, before the program is run.
pub fn set_app(app: AndroidApp) {
    let _ = APP.set(app);
}

/// The activity the program runs in, once [`set_app`] has said it.
pub fn app() -> Option<AndroidApp> {
    APP.get().cloned()
}

/// What the system's keyboard did, as the shell's `InputConnection` was told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextInput {
    /// Text that is typed for good (`commitText`).
    Commit(String),
    /// The word that is being composed (`setComposingText`); empty when
    /// there is none any more.
    Preedit(String),
}

static TYPED: Mutex<Vec<TextInput>> = Mutex::new(Vec::new());
static WAKE: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

/// Hands what the keyboard did to the window that has the focus, as the
/// input method's events of the other systems. From any thread.
pub fn text_input(input: TextInput) {
    if let Ok(mut typed) = TYPED.lock() {
        typed.push(input);
    }

    if let Some(wake) = WAKE.get() {
        wake();
    }
}

/// How the event loop is made to turn when something was typed.
pub(crate) fn set_wake(wake: Box<dyn Fn() + Send + Sync>) {
    let _ = WAKE.set(wake);
}

/// What was typed since the last call.
pub(crate) fn typed() -> Vec<TextInput> {
    TYPED
        .lock()
        .map(|mut typed| std::mem::take(&mut *typed))
        .unwrap_or_default()
}
