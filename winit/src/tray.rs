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

/// Whoever listens to the tray icon.
struct Listeners {
    /// One sender per running [`events`] subscription.
    senders: Vec<mpsc::UnboundedSender<Event>>,
    /// Whether the desktop shows the icon, as far as it is known.
    available: Option<bool>,
}

static LISTENERS: Mutex<Listeners> = Mutex::new(Listeners {
    senders: Vec::new(),
    available: None,
});

fn listeners() -> std::sync::MutexGuard<'static, Listeners> {
    LISTENERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Whether this desktop shows the tray icon of the program: the latest of
/// [`Event::Available`] and [`Event::Unavailable`]. `None` while the
/// program has no tray icon, and for the moment after it asked for one.
pub fn is_available() -> Option<bool> {
    listeners().available
}

/// What a person does with the tray icon: an item of its menu chosen
/// ([`Event::MenuItem`]), the icon itself clicked ([`Event::Click`]); and
/// whether the desktop shows the icon at all ([`Event::Available`],
/// [`Event::Unavailable`]: the latest of the two is told first).
pub fn events() -> Subscription<Event> {
    Subscription::run(listen)
}

fn listen() -> mpsc::UnboundedReceiver<Event> {
    let (sender, receiver) = mpsc::unbounded();

    let mut listeners = listeners();

    // What was found out before anybody listened
    if let Some(is_available) = listeners.available {
        let _ = sender.unbounded_send(if is_available {
            Event::Available
        } else {
            Event::Unavailable
        });
    }

    listeners.senders.push(sender);

    receiver
}

/// Hands an [`Event`] to every listener; the ones that went away are
/// forgotten. Whether the icon is shown is told only when it changes.
#[cfg_attr(
    not(all(
        feature = "tray",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )),
    allow(dead_code)
)]
pub(crate) fn emit(event: Event) {
    let mut listeners = listeners();

    let available = match event {
        Event::Available => Some(true),
        Event::Unavailable => Some(false),
        _ => None,
    };

    if available.is_some() {
        if listeners.available == available {
            return;
        }

        listeners.available = available;
    }

    listeners
        .senders
        .retain(|listener| listener.unbounded_send(event.clone()).is_ok());
}

/// Forgets whether the icon is shown: there is none any more.
#[cfg_attr(
    not(all(
        feature = "tray",
        any(target_os = "linux", target_os = "macos", target_os = "windows")
    )),
    allow(dead_code)
)]
fn forget() {
    listeners().available = None;
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
            Action::Remove => {
                self.raw = None;
                forget();
            }
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
                    None => {
                        self.raw = native::Tray::new(settings);

                        // These systems always have a place for the icon
                        emit(if self.raw.is_some() {
                            Event::Available
                        } else {
                            Event::Unavailable
                        });
                    }
                },
                Action::Remove => {
                    self.raw = None;
                    forget();
                }
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

        forget();

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
            listeners()
                .senders
                .iter()
                .all(|listener| !listener.is_closed())
        );
    }

    #[test]
    fn whether_there_is_a_tray_is_told_when_it_changes_and_to_who_comes_late() {
        let _alone = TESTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        forget();
        assert_eq!(is_available(), None);

        let mut early = listen();

        emit(Event::Unavailable);
        emit(Event::Unavailable);
        assert_eq!(is_available(), Some(false));

        // Who starts to listen now hears where things stand
        let mut late = listen();
        assert_eq!(block_on(late.next()), Some(Event::Unavailable));

        emit(Event::Available);
        emit(Event::Available);
        emit(Event::Click);
        assert_eq!(is_available(), Some(true));

        assert_eq!(block_on(early.next()), Some(Event::Unavailable));
        assert_eq!(block_on(early.next()), Some(Event::Available));
        assert_eq!(block_on(early.next()), Some(Event::Click));
        assert_eq!(block_on(late.next()), Some(Event::Available));
        assert_eq!(block_on(late.next()), Some(Event::Click));

        // The icon is taken away: nothing is known any more
        forget();
        assert_eq!(is_available(), None);
    }
}
