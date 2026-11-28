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
use std::sync::OnceLock;

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
