use iced::widget::{center, column, text};
use iced::window::{self, ActivationPolicy};
use iced::{Element, Subscription, Task};

pub fn main() -> iced::Result {
    iced::daemon(Example::new, Example::update, Example::view)
        .title("Single instance")
        .subscription(Example::subscription)
        .single_instance("com.lumcells.iced.examples.single-instance")
        // On macOS: no Dock icon until the window is open
        .activation_policy(ActivationPolicy::Accessory)
        .run()
}

struct Example {
    window: Option<window::Id>,
    starts: Vec<Vec<String>>,
}

#[derive(Debug, Clone)]
enum Message {
    /// The program was started again, with these arguments.
    StartedAgain(Vec<String>),
    Opened,
    Closed(window::Id),
}

impl Example {
    fn new() -> (Self, Task<Message>) {
        let mut example = Self {
            window: None,
            starts: Vec::new(),
        };

        let open = example.open();

        (example, open)
    }

    fn open(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            // To the front on every system: restored if it was minimized,
            // and on Wayland with the token the second start handed over
            return window::present(id);
        }

        let (id, open) = window::open(window::Settings {
            size: iced::Size::new(420.0, 260.0),
            // Where and as large as it was closed the last time (kept
            // under the identifier of the single instance)
            remember: Some(window::Remember::new("main")),
            ..window::Settings::default()
        });

        self.window = Some(id);

        Task::batch([
            window::set_activation_policy(ActivationPolicy::Regular),
            open.map(|_| Message::Opened),
        ])
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::StartedAgain(arguments) => {
                // What the test of the two processes reads
                println!("started again with {arguments:?}");

                self.starts.push(arguments);
                self.open()
            }
            Message::Opened => Task::none(),
            Message::Closed(id) => {
                if self.window == Some(id) {
                    self.window = None;

                    // Back to the tray-only look; a real program would
                    // keep a tray icon to be opened from
                    return window::set_activation_policy(
                        ActivationPolicy::Accessory,
                    )
                    .chain(iced::exit());
                }

                Task::none()
            }
        }
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let starts =
            column(self.starts.iter().enumerate().map(|(index, arguments)| {
                text!("Start {}: {arguments:?}", index + 2).into()
            }))
            .spacing(6);

        center(
            column![
                text("Start this program again, with arguments.").size(18),
                text!("Started again {} times", self.starts.len()),
                starts,
            ]
            .spacing(14),
        )
        .padding(20)
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::single_instance::listen().map(Message::StartedAgain),
            window::close_events().map(Message::Closed),
        ])
    }
}
