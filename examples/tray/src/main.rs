use iced::time::{self, Duration};
use iced::tray;
use iced::widget::{button, center, column, text};
use iced::window;
use iced::{Center, Element, Subscription, Task};

pub fn main() -> iced::Result {
    iced::daemon(Counter::new, Counter::update, Counter::view)
        .title("Tray")
        .subscription(Counter::subscription)
        .tray(Counter::default().tray())
        // macOS: a program of the menu bar. No Dock icon while it has no
        // window, in the Dock while it has one, and it does not take the
        // keyboard from the person when it starts with its icon only
        .activation_policy(window::ActivationPolicy::Accessory)
        .dock_follows_windows(true)
        .activate_at_start(false)
        // macOS: "Quit" of the menu (Command-Q) is the program's own quit
        // (Command-W closes the window by itself)
        .exit_on_quit_request(false)
        .run()
}

struct Counter {
    seconds: u64,
    /// How much a tick counts: chosen in the submenu "Count by".
    step: u64,
    paused: bool,
    window: Option<window::Id>,
    /// Whether this desktop shows tray icons, once that is known.
    has_tray: Option<bool>,
}

impl Default for Counter {
    fn default() -> Self {
        Self {
            seconds: 0,
            step: 1,
            paused: false,
            window: None,
            has_tray: None,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Tray(tray::Event),
    TogglePause,
    Opened,
    Closed(window::Id),
    QuitRequested,
}

impl Counter {
    fn new() -> (Self, Task<Message>) {
        (Self::default(), Task::none())
    }

    /// The tray icon as it should be now.
    fn tray(&self) -> tray::Settings {
        let status = if self.paused {
            format!("Paused at {} seconds", self.seconds)
        } else {
            format!("{} seconds", self.seconds)
        };

        tray::Settings::new(icon(self.paused))
            // What a screen reader (and the settings of Windows) call the
            // icon: the same whatever the status is
            .name("Tray example")
            .tooltip(format!("Tray example: {status}"))
            .menu([
                // Not every desktop shows a tooltip: the status is the
                // first line of the menu too
                tray::Item::label(status),
                tray::Item::separator(),
                tray::Item::button("open", "Open"),
                tray::Item::check("pause", "Pause", self.paused),
                // A menu inside the menu, with a picture before each item
                // (and before itself)
                tray::Item::submenu(
                    "Count by",
                    STEPS.map(|step| {
                        tray::Item::button(
                            format!("step-{step}"),
                            if step == 1 {
                                "1 second".to_owned()
                            } else {
                                format!("{step} seconds")
                            },
                        )
                        .icon(dot(step == self.step))
                    }),
                )
                .icon(dot(true)),
                tray::Item::separator(),
                tray::Item::button("quit", "Quit"),
            ])
    }

    fn open(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            // To the front wherever it is: behind other windows,
            // minimized, on Wayland too (with the token of a second
            // start, where there is one)
            return window::present(id);
        }

        let (id, open) = window::open(window::Settings {
            size: iced::Size::new(360.0, 240.0),
            // The window comes back where it was closed, as large as it was
            remember: Some(window::Remember::new("main")),
            ..window::Settings::default()
        });

        self.window = Some(id);

        open.map(|_| Message::Opened)
    }

    fn toggle_pause(&mut self) -> Task<Message> {
        self.paused = !self.paused;

        tray::update(self.tray())
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => {
                self.seconds += self.step;

                tray::update(self.tray())
            }
            Message::TogglePause => self.toggle_pause(),
            Message::Tray(tray::Event::MenuItem(id)) => match id.as_str() {
                "open" => self.open(),
                "pause" => self.toggle_pause(),
                "quit" => iced::exit(),
                id => match id
                    .strip_prefix("step-")
                    .and_then(|step| step.parse().ok())
                {
                    Some(step) => {
                        self.step = step;

                        tray::update(self.tray())
                    }
                    None => Task::none(),
                },
            },
            Message::Tray(tray::Event::Click) => self.open(),
            // No tray on this desktop (GNOME without the extension, a
            // bare window manager): nobody can reach the icon, so the
            // window is opened and says so
            Message::Tray(tray::Event::Unavailable) => {
                self.has_tray = Some(false);

                self.open()
            }
            Message::Tray(tray::Event::Available) => {
                self.has_tray = Some(true);

                Task::none()
            }
            // Here a program finishes what it is doing; then it ends
            Message::QuitRequested => iced::exit(),
            Message::Opened => Task::none(),
            Message::Closed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }

                // Without a tray a program with no window could never be
                // reached again: it ends with its window
                if self.window.is_none() && self.has_tray == Some(false) {
                    iced::exit()
                } else {
                    Task::none()
                }
            }
        }
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        center(
            column![
                text!("{} seconds", self.seconds).size(28),
                button(if self.paused { "Resume" } else { "Pause" })
                    .on_press(Message::TogglePause),
                text(match self.has_tray {
                    Some(false) => {
                        "This desktop shows no tray icons: closing this \
                        window ends the program."
                    }
                    _ => "Closing this window leaves the icon in the tray.",
                })
                .size(12),
            ]
            .spacing(16)
            .align_x(Center),
        )
        .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let ticks = if self.paused {
            Subscription::none()
        } else {
            time::every(Duration::from_secs(1)).map(|_| Message::Tick)
        };

        Subscription::batch([
            ticks,
            tray::events().map(Message::Tray),
            window::close_events().map(Message::Closed),
            iced::system::quit_requests().map(|()| Message::QuitRequested),
        ])
    }
}

/// A disc of 32 pixels: blue while counting, grey while paused.
fn icon(paused: bool) -> tray::Icon {
    const SIZE: u32 = 32;

    let [red, green, blue] = if paused {
        [0x8a, 0x8f, 0x98]
    } else {
        [0x2f, 0x6f, 0xed]
    };

    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - SIZE as f32 / 2.0;
            let dy = y as f32 + 0.5 - SIZE as f32 / 2.0;
            let distance = (dx * dx + dy * dy).sqrt();

            // One pixel of soft edge
            let alpha = (SIZE as f32 / 2.0 - 1.0 - distance).clamp(0.0, 1.0);

            // The two bars of "pause", or nothing
            let is_bar = paused
                && (10..22).contains(&y)
                && ((11..14).contains(&x) || (18..21).contains(&x));

            if is_bar {
                rgba.extend([0xff, 0xff, 0xff, 0xff]);
            } else {
                rgba.extend([red, green, blue, (alpha * 255.0) as u8]);
            }
        }
    }

    tray::Icon::from_rgba(rgba, SIZE, SIZE).expect("32x32 pixels of RGBA")
}

/// What a tick can count.
const STEPS: [u64; 3] = [1, 5, 60];

/// The picture of an item of the menu: a dot of 16 pixels, blue for what
/// is chosen and grey for the rest.
fn dot(chosen: bool) -> tray::Icon {
    const SIZE: u32 = 16;

    let [red, green, blue] = if chosen {
        [0x2f, 0x6f, 0xed]
    } else {
        [0xb0, 0xb4, 0xba]
    };

    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 + 0.5 - SIZE as f32 / 2.0;
            let dy = y as f32 + 0.5 - SIZE as f32 / 2.0;
            let alpha = (5.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);

            rgba.extend([red, green, blue, (alpha * 255.0) as u8]);
        }
    }

    tray::Icon::from_rgba(rgba, SIZE, SIZE).expect("16x16 pixels of RGBA")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(settings: &tray::Settings) -> Vec<&str> {
        settings
            .menu
            .items
            .iter()
            .map(|item| match item {
                tray::Item::Button { label, .. }
                | tray::Item::Submenu { label, .. } => label.as_str(),
                tray::Item::Separator => "-",
            })
            .collect()
    }

    #[test]
    fn the_tray_tells_the_count_and_offers_open_pause_quit() {
        let mut counter = Counter::default();
        let _ = counter.update(Message::Tick);
        let _ = counter.update(Message::Tick);

        let settings = counter.tray();
        assert_eq!(settings.tooltip, "Tray example: 2 seconds");
        assert_eq!(settings.title(), "Tray example");
        assert_eq!(
            labels(&settings),
            ["2 seconds", "-", "Open", "Pause", "Count by", "-", "Quit"]
        );
    }

    #[test]
    fn the_submenu_chooses_what_a_tick_counts_and_marks_it_with_a_picture() {
        let mut counter = Counter::default();

        let inside = |counter: &Counter| match &counter.tray().menu.items[4] {
            tray::Item::Submenu {
                menu,
                icon: Some(_),
                ..
            } => menu.clone(),
            other => panic!("not a submenu with a picture: {other:?}"),
        };

        let chosen = |counter: &Counter| -> Vec<bool> {
            inside(counter)
                .items
                .iter()
                .map(|item| match item {
                    tray::Item::Button {
                        icon: Some(icon), ..
                    } => *icon == dot(true),
                    other => panic!("no picture: {other:?}"),
                })
                .collect()
        };

        assert_eq!(chosen(&counter), [true, false, false]);

        let _ = counter
            .update(Message::Tray(tray::Event::MenuItem("step-5".into())));
        assert_eq!(chosen(&counter), [false, true, false]);

        let _ = counter.update(Message::Tick);
        assert_eq!(counter.seconds, 5);
    }

    #[test]
    fn without_a_tray_the_window_opens_and_the_program_ends_with_it() {
        let mut counter = Counter::default();

        // The desktop shows no tray icons: the window is opened, since
        // nothing else of the program can be reached
        let _ = counter.update(Message::Tray(tray::Event::Unavailable));
        let window = counter.window.expect("a window");
        assert_eq!(counter.has_tray, Some(false));

        // A panel is started: the icon is there, the window may go
        let _ = counter.update(Message::Tray(tray::Event::Available));
        assert_eq!(counter.has_tray, Some(true));
        assert_eq!(counter.window, Some(window));
    }

    #[test]
    fn pause_from_the_menu_ticks_the_item_and_changes_the_icon() {
        let mut counter = Counter::default();
        let before = counter.tray();

        let _ = counter
            .update(Message::Tray(tray::Event::MenuItem("pause".into())));
        assert!(counter.paused);

        let after = counter.tray();
        assert_ne!(before.icon, after.icon);
        assert_eq!(
            after.menu.items[3],
            tray::Item::check("pause", "Pause", true)
        );
        assert_eq!(after.tooltip, "Tray example: Paused at 0 seconds");

        let _ = counter
            .update(Message::Tray(tray::Event::MenuItem("pause".into())));
        assert!(!counter.paused);
    }

    #[test]
    fn a_click_on_the_icon_opens_one_window() {
        let mut counter = Counter::default();

        let _ = counter.update(Message::Tray(tray::Event::Click));
        let first = counter.window;
        assert!(first.is_some());

        let _ =
            counter.update(Message::Tray(tray::Event::MenuItem("open".into())));
        assert_eq!(counter.window, first, "the same window, not a second");

        let _ = counter.update(Message::Closed(first.unwrap()));
        assert_eq!(counter.window, None);
    }
}
