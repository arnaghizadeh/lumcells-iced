//! Listen to a channel that the application made itself.
use crate::futures::channel::mpsc;
use crate::futures::stream::{self, Stream, StreamExt};
use crate::subscription::{self, Hasher, Subscription};
use crate::{BoxStream, MaybeSend};

use std::hash::Hash;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

/// A channel whose receiving end is owned by a [`Subscription`].
///
/// [`Subscription::run`] takes a function pointer, so a stream that needs
/// something made at run time (the receiver of a channel that a thread of
/// the application writes to) has nowhere to take it from but a global.
/// An [`Inbox`] is that channel and its subscription in one value: keep it
/// in the state of the application, give [`Inbox::sender`] to whoever
/// produces messages (any thread), and return [`Inbox::subscription`] from
/// `subscription`.
///
/// ```
/// use iced_futures::Subscription;
/// use iced_futures::subscription::Inbox;
///
/// struct App {
///     inbox: Inbox<Message>,
/// }
///
/// #[derive(Debug, Clone)]
/// enum Message {
///     Progress(u8),
/// }
///
/// impl App {
///     fn new() -> Self {
///         let inbox = Inbox::new();
///         let sender = inbox.sender();
///
///         let _ = std::thread::spawn(move || {
///             sender.send(Message::Progress(50));
///         });
///
///         Self { inbox }
///     }
///
///     fn subscription(&self) -> Subscription<Message> {
///         self.inbox.subscription()
///     }
/// }
/// ```
///
/// What is sent while no subscription listens waits in the channel. The
/// subscription can be dropped and returned again later: the receiver goes
/// back into the [`Inbox`] in between.
#[derive(Debug)]
pub struct Inbox<T> {
    id: u64,
    sender: mpsc::UnboundedSender<T>,
    receiver: Slot<T>,
}

type Slot<T> = Arc<Mutex<Option<mpsc::UnboundedReceiver<T>>>>;

impl<T> Inbox<T> {
    /// Creates a new [`Inbox`].
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);

        let (sender, receiver) = mpsc::unbounded();

        Self {
            id: NEXT.fetch_add(1, Ordering::Relaxed),
            sender,
            receiver: Arc::new(Mutex::new(Some(receiver))),
        }
    }

    /// Returns a [`Sender`] that puts messages in the [`Inbox`]; it can be
    /// cloned and moved to other threads.
    pub fn sender(&self) -> Sender<T> {
        Sender(self.sender.clone())
    }

    /// Returns the [`Subscription`] that produces what is put in the
    /// [`Inbox`].
    pub fn subscription(&self) -> Subscription<T>
    where
        T: MaybeSend + 'static,
    {
        subscription::from_recipe(Recipe {
            id: self.id,
            receiver: self.receiver.clone(),
        })
    }
}

impl<T> Default for Inbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// The sending end of an [`Inbox`].
#[derive(Debug)]
pub struct Sender<T>(mpsc::UnboundedSender<T>);

impl<T> Sender<T> {
    /// Puts a message in the [`Inbox`]. Returns `false` if the [`Inbox`]
    /// is gone.
    pub fn send(&self, message: T) -> bool {
        self.0.unbounded_send(message).is_ok()
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct Recipe<T> {
    id: u64,
    receiver: Slot<T>,
}

impl<T> subscription::Recipe for Recipe<T>
where
    T: MaybeSend + 'static,
{
    type Output = T;

    fn hash(&self, state: &mut Hasher) {
        std::any::TypeId::of::<Self>().hash(state);
        self.id.hash(state);
    }

    fn stream(
        self: Box<Self>,
        _input: subscription::EventStream,
    ) -> BoxStream<Self::Output> {
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();

        match receiver {
            Some(receiver) => crate::boxed_stream(Listening {
                receiver: Some(receiver),
                slot: self.receiver,
            }),
            // Another subscription listens already
            None => crate::boxed_stream(stream::pending()),
        }
    }
}

/// The stream of a subscription to an [`Inbox`]: it gives the receiver
/// back when it is dropped.
struct Listening<T> {
    receiver: Option<mpsc::UnboundedReceiver<T>>,
    slot: Slot<T>,
}

impl<T> Stream for Listening<T> {
    type Item = T;

    fn poll_next(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<T>> {
        match self.receiver.as_mut() {
            Some(receiver) => receiver.poll_next_unpin(context),
            None => Poll::Ready(None),
        }
    }
}

impl<T> Unpin for Listening<T> {}

impl<T> Drop for Listening<T> {
    fn drop(&mut self) {
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) =
            self.receiver.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::futures::executor::block_on;
    use crate::subscription::into_recipes;

    fn stream<T: MaybeSend + 'static>(inbox: &Inbox<T>) -> BoxStream<T> {
        let recipe = into_recipes(inbox.subscription()).pop().unwrap();

        recipe.stream(crate::boxed_stream(stream::pending()))
    }

    fn identity<T: MaybeSend + 'static>(inbox: &Inbox<T>) -> u64 {
        use std::hash::Hasher as _;

        let recipe = into_recipes(inbox.subscription()).pop().unwrap();
        let mut hasher = Hasher::default();
        recipe.hash(&mut hasher);
        hasher.finish()
    }

    #[test]
    fn what_a_thread_sends_comes_out_of_the_subscription() {
        let inbox = Inbox::new();
        let sender = inbox.sender();

        // Sent before anything listens: it waits
        assert!(sender.send(1));

        let thread = std::thread::spawn(move || {
            assert!(sender.send(2));
            assert!(sender.send(3));
        });
        thread.join().unwrap();

        let mut messages = stream(&inbox);

        assert_eq!(block_on(messages.next()), Some(1));
        assert_eq!(block_on(messages.next()), Some(2));
        assert_eq!(block_on(messages.next()), Some(3));
    }

    #[test]
    fn the_subscription_is_the_same_each_time_and_differs_per_inbox() {
        let first = Inbox::<u8>::new();
        let second = Inbox::<u8>::new();

        assert_eq!(identity(&first), identity(&first));
        assert_ne!(identity(&first), identity(&second));
    }

    #[test]
    fn a_subscription_that_was_dropped_can_listen_again() {
        let inbox = Inbox::new();
        let sender = inbox.sender();

        let mut messages = stream(&inbox);
        assert!(sender.send("first"));
        assert_eq!(block_on(messages.next()), Some("first"));
        drop(messages);

        assert!(sender.send("second"));
        let mut messages = stream(&inbox);
        assert_eq!(block_on(messages.next()), Some("second"));
    }

    #[test]
    fn sending_to_an_inbox_that_is_gone_says_so() {
        let inbox = Inbox::new();
        let sender = inbox.sender();
        assert!(sender.send(()));

        drop(inbox);
        assert!(!sender.send(()));
    }
}
