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
        .run()
}

#[derive(Default)]
struct Counter {
    seconds: u64,
    paused: bool,
    window: Option<window::Id>,
}

#[derive(Debug, Clone)]
enum Message {
    Tick,
    Tray(tray::Event),
    TogglePause,
    Opened,
    Closed(window::Id),
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
            .tooltip(format!("Tray example: {status}"))
            .menu([
                // Not every desktop shows a tooltip: the status is the
                // first line of the menu too
                tray::Item::label(status),
                tray::Item::separator(),
                tray::Item::button("open", "Open"),
                tray::Item::check("pause", "Pause", self.paused),
                tray::Item::separator(),
                tray::Item::button("quit", "Quit"),
            ])
    }

    fn open(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            return window::gain_focus(id);
        }

        let (id, open) = window::open(window::Settings {
            size: iced::Size::new(320.0, 200.0),
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
                self.seconds += 1;

                tray::update(self.tray())
            }
            Message::TogglePause => self.toggle_pause(),
            Message::Tray(tray::Event::MenuItem(id)) => match id.as_str() {
                "open" => self.open(),
                "pause" => self.toggle_pause(),
                "quit" => iced::exit(),
                _ => Task::none(),
            },
            Message::Tray(tray::Event::Click) => self.open(),
            Message::Opened => Task::none(),
            Message::Closed(id) => {
                if self.window == Some(id) {
                    self.window = None;
                }

                Task::none()
            }
        }
    }

    fn view(&self, _window: window::Id) -> Element<'_, Message> {
        center(
            column![
                text!("{} seconds", self.seconds).size(28),
                button(if self.paused { "Resume" } else { "Pause" })
                    .on_press(Message::TogglePause),
                text("Closing this window leaves the icon in the tray.")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(settings: &tray::Settings) -> Vec<&str> {
        settings
            .menu
            .items
            .iter()
            .map(|item| match item {
                tray::Item::Button { label, .. } => label.as_str(),
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
        assert_eq!(
            labels(&settings),
            ["2 seconds", "-", "Open", "Pause", "-", "Quit"]
        );
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
