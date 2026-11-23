//! Show an icon with a menu in the tray of the desktop.
//!
//! - Linux: the `org.kde.StatusNotifierItem` and `com.canonical.dbusmenu`
//!   interfaces, served directly on the session bus. No GTK, no thread of
//!   its own, no polling: the connection is driven by the executor of the
//!   program.
//! - macOS and Windows: the `tray-icon` crate, inside the event loop.
//!
//! Give the icon to the program when it starts (`iced::daemon(..).tray(..)`),
//! change it with [`update`], and listen to it with [`events`].
pub use crate::runtime::tray::{
    Action, Event, Icon, IconError, Item, Menu, Settings, remove, update,
};

use crate::futures::Subscription;
use crate::futures::futures::channel::mpsc;
use crate::futures::futures::stream::BoxStream;

use std::sync::Mutex;

#[cfg(all(feature = "tray", target_os = "linux"))]
mod linux;

#[cfg(all(feature = "tray", any(target_os = "macos", target_os = "windows")))]
mod native;

/// Whoever listens to the tray icon: one sender per running [`events`]
/// subscription.
static LISTENERS: Mutex<Vec<mpsc::UnboundedSender<Event>>> =
    Mutex::new(Vec::new());

/// What a person does with the tray icon: an item of its menu chosen
/// ([`Event::MenuItem`]), the icon itself clicked ([`Event::Click`]).
pub fn events() -> Subscription<Event> {
    Subscription::run(listen)
}

fn listen() -> mpsc::UnboundedReceiver<Event> {
    let (sender, receiver) = mpsc::unbounded();

    LISTENERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(sender);

    receiver
}

/// Hands an [`Event`] to every listener; the ones that went away are
/// forgotten.
#[cfg_attr(
    not(all(
        feature = "tray",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )),
    allow(dead_code)
)]
pub(crate) fn emit(event: Event) {
    LISTENERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|listener| listener.unbounded_send(event.clone()).is_ok());
}

/// The listeners are shared by the whole process: the tests that emit
/// events run one at a time.
#[cfg(test)]
static TESTS: Mutex<()> = Mutex::new(());

/// The tray icon of a running program, if it has one.
#[derive(Default)]
pub(crate) struct Tray {
    #[cfg(all(feature = "tray", target_os = "linux"))]
    raw: Option<linux::Tray>,
    #[cfg(all(
        feature = "tray",
        any(target_os = "macos", target_os = "windows")
    ))]
    raw: Option<native::Tray>,
}

impl Tray {
    /// Performs a tray [`Action`]. Has to be called from the thread of the
    /// event loop, once it runs.
    ///
    /// `spawn` runs a stream on the executor of the program until it ends
    /// (the stream yields nothing).
    pub fn perform(
        &mut self,
        action: Action,
        spawn: impl FnOnce(BoxStream<'static, ()>),
    ) {
        #[cfg(all(feature = "tray", target_os = "linux"))]
        match action {
            Action::Update(settings) => match &mut self.raw {
                Some(tray) if tray.is_alive() => tray.update(settings),
                _ => self.raw = Some(linux::Tray::new(settings, spawn)),
            },
            Action::Remove => self.raw = None,
        }

        #[cfg(all(
            feature = "tray",
            any(target_os = "macos", target_os = "windows")
        ))]
        {
            let _ = spawn;

            match action {
                Action::Update(settings) => match &mut self.raw {
                    Some(tray) => tray.update(settings),
                    None => self.raw = native::Tray::new(settings),
                },
                Action::Remove => self.raw = None,
            }
        }

        #[cfg(not(all(
            feature = "tray",
            any(
                target_os = "linux",
                target_os = "macos",
                target_os = "windows"
            )
        )))]
        {
            let _ = (action, spawn);

            log::warn!(
                "the tray icon is not available: the `tray` feature is \
                off, or this platform has no tray"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::futures::futures::StreamExt;
    use crate::futures::futures::executor::block_on;

    #[test]
    fn every_listener_hears_an_event_and_the_gone_are_forgotten() {
        let _alone = TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut first = listen();
        let second = listen();
        drop(second);

        emit(Event::Click);
        emit(Event::MenuItem("quit".to_owned()));

        assert_eq!(block_on(first.next()), Some(Event::Click));
        assert_eq!(
            block_on(first.next()),
            Some(Event::MenuItem("quit".to_owned()))
        );

        // The listener that was dropped is no longer kept
        assert!(
            LISTENERS
                .lock()
                .unwrap()
                .iter()
                .all(|listener| !listener.is_closed())
        );
    }
}
