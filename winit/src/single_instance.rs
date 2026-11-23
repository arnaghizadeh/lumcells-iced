//! Run one instance of a program per user.
//!
//! With `iced::daemon(..).single_instance("com.example.app")`, the first
//! process of a user listens on a socket of that user (a Unix socket in the
//! runtime directory; a named pipe on Windows). A process started while it
//! runs hands it its arguments over that socket and ends at once, before
//! any window or event loop is made. The first process receives the
//! arguments through [`listen`] and does what a second start asks for:
//! it opens its window, or brings it to the front.
//!
//! On Wayland a program may only raise a window with an activation token
//! of the desktop. The second process passes the token it was started with
//! (`XDG_ACTIVATION_TOKEN`) through the same socket, and the next window
//! the first process opens is given it. (A window that is already open
//! cannot be raised on Wayland with the version of `winit` in use; on X11,
//! Windows and macOS `window::gain_focus` does it.)
use crate::futures::Subscription;
use crate::futures::futures::channel::mpsc;

use std::sync::Mutex;

/// The arguments of every later start of the program (without the name of
/// the program), in the first instance.
///
/// Starts that came before anything listened are kept and delivered first.
/// In a program that did not ask for a single instance, nothing comes.
pub fn listen() -> Subscription<Vec<String>> {
    Subscription::run(receiver)
}

/// What became of a start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// This process is the instance: it listens for later starts.
    First,
    /// Another process is: it was handed the arguments, and this process
    /// has nothing more to do.
    Forwarded,
}

struct State {
    listeners: Vec<mpsc::UnboundedSender<Vec<String>>>,
    /// The starts nobody listened to yet.
    backlog: Vec<Vec<String>>,
    /// The activation token of the latest start, for the next window.
    token: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    listeners: Vec::new(),
    backlog: Vec::new(),
    token: None,
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn receiver() -> mpsc::UnboundedReceiver<Vec<String>> {
    let (sender, receiver) = mpsc::unbounded();
    let mut state = state();

    for arguments in state.backlog.drain(..) {
        let _ = sender.unbounded_send(arguments);
    }

    state.listeners.push(sender);

    receiver
}

#[cfg_attr(not(feature = "single-instance"), allow(dead_code))]
fn deliver(message: Message) {
    let mut state = state();

    if message.token.is_some() {
        state.token = message.token;
    }

    state.listeners.retain(|listener| {
        listener.unbounded_send(message.arguments.clone()).is_ok()
    });

    if state.listeners.is_empty() {
        state.backlog.push(message.arguments);
    }
}

/// Takes the activation token that waits for the next window, if any.
#[cfg_attr(
    not(all(
        unix,
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))
    )),
    allow(dead_code)
)]
pub(crate) fn take_activation_token() -> Option<String> {
    state().token.take()
}

/// What a start hands over.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Message {
    token: Option<String>,
    arguments: Vec<String>,
}

#[cfg_attr(not(feature = "single-instance"), allow(dead_code))]
impl Message {
    /// The most a message may weigh; more is not a message of ours.
    const LIMIT: usize = 1 << 20;

    /// A count of fields, then each field as its length and its bytes
    /// (numbers as four bytes, little end first). The first field is the
    /// token, empty for none.
    fn encode(&self) -> Vec<u8> {
        let fields = std::iter::once(self.token.as_deref().unwrap_or(""))
            .chain(self.arguments.iter().map(String::as_str));

        let mut bytes =
            (self.arguments.len() as u32 + 1).to_le_bytes().to_vec();

        for field in fields {
            bytes.extend((field.len() as u32).to_le_bytes());
            bytes.extend(field.as_bytes());
        }

        bytes
    }

    fn read(reader: &mut impl std::io::Read) -> std::io::Result<Self> {
        use std::io::{Error, ErrorKind};

        let number = |reader: &mut dyn std::io::Read| {
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes)?;
            Ok::<_, Error>(u32::from_le_bytes(bytes) as usize)
        };

        let count = number(reader)?;
        let mut total = 0;
        let mut fields = Vec::new();

        for _ in 0..count {
            let length = number(reader)?;
            total += length + 4;

            if total > Self::LIMIT {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "the message is too long",
                ));
            }

            let mut field = vec![0; length];
            reader.read_exact(&mut field)?;

            fields.push(String::from_utf8(field).map_err(|_| {
                Error::new(ErrorKind::InvalidData, "not UTF-8")
            })?);
        }

        let mut fields = fields.into_iter();
        let token = fields.next().filter(|token| !token.is_empty());

        Ok(Self {
            token,
            arguments: fields.collect(),
        })
    }
}

/// Makes this process the instance of the program with the given
/// identifier, or hands its arguments to the one that is.
///
/// Where the socket cannot be made at all, the process runs as if it were
/// the first: a program that starts twice is better than one that does not.
#[cfg(feature = "single-instance")]
pub fn start(id: &str) -> Start {
    // The token of the desktop for this start
    let token = std::env::var("XDG_ACTIVATION_TOKEN")
        .ok()
        .filter(|token| !token.is_empty());

    let message = Message {
        token,
        arguments: std::env::args().skip(1).collect(),
    };

    match socket::start(&socket::Address::of(id), &message, deliver) {
        Ok(Start::Forwarded) => Start::Forwarded,
        Ok(Start::First) => {
            // The window of the first start may use its own token
            state().token = message.token;

            Start::First
        }
        Err(error) => {
            log::warn!(
                "no single instance: the socket of `{id}` could not be \
                made ({error})"
            );

            Start::First
        }
    }
}

#[cfg(feature = "single-instance")]
mod socket {
    use super::{Message, Start};

    use interprocess::local_socket::prelude::*;
    use interprocess::local_socket::{ListenerOptions, Name, Stream};

    use std::io::{self, Read, Write};

    /// Where the instance of a program listens.
    #[derive(Debug, Clone)]
    pub struct Address {
        /// On Unix: the path of the socket.
        #[cfg(unix)]
        path: std::path::PathBuf,
        /// On Windows: the name of the pipe.
        #[cfg(not(unix))]
        pipe: String,
    }

    impl Address {
        /// The address of the given identifier for the user of this process.
        pub fn of(id: &str) -> Self {
            let id: String = id
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || "._-".contains(c) {
                        c
                    } else {
                        '_'
                    }
                })
                .collect();

            #[cfg(unix)]
            {
                // The runtime directory belongs to the user alone; the
                // temporary one (macOS gives each user one) is shared on
                // other systems, so the name of the user goes in the name
                let path = match std::env::var_os("XDG_RUNTIME_DIR") {
                    Some(directory) if !directory.is_empty() => {
                        std::path::PathBuf::from(directory)
                            .join(format!("{id}.sock"))
                    }
                    _ => std::env::temp_dir().join(format!(
                        "{id}-{}.sock",
                        std::env::var("USER").unwrap_or_default()
                    )),
                };

                Self { path }
            }

            #[cfg(not(unix))]
            {
                Self {
                    pipe: format!(
                        "{id}-{}",
                        std::env::var("USERNAME").unwrap_or_default()
                    ),
                }
            }
        }

        /// An address in the given directory, for tests.
        #[cfg(all(test, unix))]
        pub fn in_directory(directory: &std::path::Path, id: &str) -> Self {
            Self {
                path: directory.join(format!("{id}.sock")),
            }
        }

        fn name(&self) -> io::Result<Name<'_>> {
            #[cfg(unix)]
            {
                use interprocess::local_socket::{GenericFilePath, ToFsName};

                self.path.as_path().to_fs_name::<GenericFilePath>()
            }

            #[cfg(not(unix))]
            {
                use interprocess::local_socket::{GenericNamespaced, ToNsName};

                self.pipe.as_str().to_ns_name::<GenericNamespaced>()
            }
        }

        /// Removes what a process that ended without cleaning left behind.
        fn clear(&self) {
            #[cfg(unix)]
            let _ = std::fs::remove_file(&self.path);
        }
    }

    /// Hands the message to the instance that listens at the address.
    fn forward(address: &Address, message: &Message) -> io::Result<()> {
        let mut stream = Stream::connect(address.name()?)?;

        stream.write_all(&message.encode())?;
        stream.flush()?;

        // The instance answers when it has the message
        let mut answer = [0; 1];
        stream.read_exact(&mut answer)?;

        Ok(())
    }

    /// Forwards the message to the instance at the address, or becomes it:
    /// then a thread listens there and gives every message to `deliver`.
    pub fn start(
        address: &Address,
        message: &Message,
        deliver: impl Fn(Message) + Send + 'static,
    ) -> io::Result<Start> {
        let mut last_error = None;

        // Two starts at the same moment: one of them binds, the other one
        // finds it at its next try
        for attempt in 0..3 {
            if forward(address, message).is_ok() {
                return Ok(Start::Forwarded);
            }

            if attempt > 0 {
                // Nobody answers there: what is in the way is left over
                address.clear();
            }

            match ListenerOptions::new().name(address.name()?).create_sync() {
                Ok(listener) => {
                    let address = address.clone();

                    let _ = std::thread::Builder::new()
                        .name("iced-single-instance".to_owned())
                        .spawn(move || {
                            let _address = Cleanup(address);

                            for stream in listener.incoming().flatten() {
                                serve(stream, &deliver);
                            }
                        })?;

                    return Ok(Start::First);
                }
                Err(error) => last_error = Some(error),
            }
        }

        Err(last_error
            .unwrap_or_else(|| io::Error::other("the socket is taken")))
    }

    fn serve(mut stream: Stream, deliver: &impl Fn(Message)) {
        match Message::read(&mut stream) {
            Ok(message) => {
                deliver(message);

                let _ = stream.write_all(&[1]);
                let _ = stream.flush();
            }
            Err(error) => {
                log::warn!("a start that could not be read: {error}");
            }
        }
    }

    struct Cleanup(Address);

    impl Drop for Cleanup {
        fn drop(&mut self) {
            self.0.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(token: Option<&str>, arguments: &[&str]) -> Message {
        Message {
            token: token.map(str::to_owned),
            arguments: arguments.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn a_message_survives_the_wire() {
        for original in [
            message(None, &[]),
            message(Some("token-1"), &["--open", "/home/ada/a file.txt"]),
            message(None, &["", "line\nbreak", "naïve ☃"]),
        ] {
            let bytes = original.encode();
            let read = Message::read(&mut bytes.as_slice()).unwrap();

            assert_eq!(read, original);
        }
    }

    #[test]
    fn what_is_not_a_message_is_refused() {
        // Cut short
        let bytes = message(None, &["abc"]).encode();
        assert!(Message::read(&mut &bytes[..bytes.len() - 1]).is_err());

        // A length beyond the limit
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend(u32::MAX.to_le_bytes());
        assert!(Message::read(&mut bytes.as_slice()).is_err());

        // Not text
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend([0xff, 0xfe]);
        assert!(Message::read(&mut bytes.as_slice()).is_err());
    }

    #[cfg(all(feature = "single-instance", unix))]
    #[test]
    fn a_second_start_hands_its_arguments_to_the_first() {
        use std::sync::mpsc;

        let directory = std::env::temp_dir()
            .join(format!("iced-single-instance-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        let address = socket::Address::in_directory(&directory, "first");
        let (sender, received) = mpsc::channel();

        // The first start becomes the instance
        let first = socket::start(&address, &message(None, &["first"]), {
            let sender = sender.clone();
            move |message| sender.send(message).unwrap()
        })
        .unwrap();
        assert_eq!(first, Start::First);
        assert!(received.try_recv().is_err(), "its own arguments stay");

        // The second and the third are forwarded, each with its arguments
        for arguments in [vec!["--open", "a b"], vec![]] {
            let start = socket::start(
                &address,
                &message(Some("token"), &arguments),
                |_| panic!("the second start does not listen"),
            )
            .unwrap();

            assert_eq!(start, Start::Forwarded);
            assert_eq!(
                received
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap(),
                message(Some("token"), &arguments)
            );
        }

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[cfg(all(feature = "single-instance", unix))]
    #[test]
    fn a_socket_left_behind_is_taken_over() {
        let directory = std::env::temp_dir()
            .join(format!("iced-single-instance-stale-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();

        // What a process that was killed leaves: a socket nobody listens on
        let path = directory.join("stale.sock");
        drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
        assert!(path.exists());

        let address = socket::Address::in_directory(&directory, "stale");
        let start =
            socket::start(&address, &message(None, &[]), |_| {}).unwrap();
        assert_eq!(start, Start::First);

        let again =
            socket::start(&address, &message(None, &["x"]), |_| {}).unwrap();
        assert_eq!(again, Start::Forwarded);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn starts_that_nobody_heard_are_kept_for_the_first_listener() {
        use crate::futures::futures::StreamExt;
        use crate::futures::futures::executor::block_on;

        deliver(message(Some("abc"), &["early"]));

        let mut first = receiver();
        assert_eq!(block_on(first.next()), Some(vec!["early".to_owned()]));

        deliver(message(None, &["late"]));
        assert_eq!(block_on(first.next()), Some(vec!["late".to_owned()]));

        // The token of the start waits for the next window, once
        assert_eq!(take_activation_token().as_deref(), Some("abc"));
        assert_eq!(take_activation_token(), None);
    }
}
