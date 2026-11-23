use iced::subscription::Inbox;
use iced::widget::{
    button, column, container, operation, row, terminal, text, text_input,
};
use iced::{Center, Element, Fill, Subscription, Task};

use std::io::{Read, Write};

pub fn main() -> iced::Result {
    iced::application(Example::with_shell, Example::update, Example::view)
        .title("Terminal")
        .subscription(Example::subscription)
        .window_size((860.0, 560.0))
        .run()
}

/// The shell and the two ends of its pseudo-terminal.
struct Shell {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

struct Example {
    content: terminal::Content,
    shell: Result<Shell, String>,
    inbox: Inbox<Message>,
    ended: bool,
    /// The search field is shown.
    searching: bool,
    /// The link that was clicked last.
    link: Option<String>,
}

const SEARCH: &str = "search";

#[derive(Debug, Clone)]
enum Message {
    /// Ctrl+Shift+F in the terminal, or "Close": the search field comes or
    /// goes.
    SearchToggled,
    /// What is searched for.
    Search(String),
    /// To the place before (Enter in the field) or after.
    SearchPrevious,
    SearchNext,
    /// A link was clicked (with Ctrl held).
    Link(String),
    /// What the shell wrote.
    Output(Vec<u8>),
    /// What was typed or pasted.
    Typed(Vec<u8>),
    /// How many cells fit in the window now.
    Fit(u16, u16),
    /// The shell is gone.
    Ended,
}

const COLS: u16 = 100;
const ROWS: u16 = 30;

fn start(inbox: &Inbox<Message>) -> Result<Shell, String> {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pair = native_pty_system()
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| error.to_string())?;

    let mut command = CommandBuilder::new_default_prog();
    command.env("TERM", "xterm-256color");

    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| error.to_string())?;
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| error.to_string())?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| error.to_string())?;

    // What the shell writes goes to the window through the inbox
    let to_window = inbox.sender();
    let _ = std::thread::spawn(move || {
        let mut buffer = [0; 8192];

        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if !to_window
                        .send(Message::Output(buffer[..count].to_vec()))
                    {
                        break;
                    }
                }
            }
        }

        let _ = to_window.send(Message::Ended);
    });

    Ok(Shell {
        master: pair.master,
        writer,
        _child: child,
    })
}

impl Example {
    fn with_shell() -> Self {
        let inbox = Inbox::new();
        let shell = start(&inbox);

        Self {
            content: terminal::Content::new(COLS, ROWS),
            shell,
            inbox,
            ended: false,
            searching: false,
            link: None,
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SearchToggled => {
                self.searching = !self.searching;

                if self.searching {
                    return operation::focus(SEARCH);
                }

                self.content.search_clear();
            }
            Message::Search(needle) => {
                let _ = self.content.search(&needle);
            }
            Message::SearchPrevious => self.content.search_previous(),
            Message::SearchNext => self.content.search_next(),
            // An application would open it; the example only says which
            Message::Link(url) => self.link = Some(url),
            Message::Output(bytes) => {
                self.content.feed(&bytes);

                // A program that asks where the cursor is gets its answer
                let answers = self.content.take_answers();

                if !answers.is_empty()
                    && let Ok(shell) = &mut self.shell
                {
                    let _ = shell.writer.write_all(&answers);
                }
            }
            Message::Typed(bytes) => {
                if let Ok(shell) = &mut self.shell {
                    let _ = shell.writer.write_all(&bytes);
                    let _ = shell.writer.flush();
                }
            }
            Message::Fit(cols, rows) => {
                self.content.resize(cols, rows);

                if let Ok(shell) = &self.shell {
                    let _ = shell.master.resize(portable_pty::PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            Message::Ended => self.ended = true,
        }

        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        self.inbox.subscription()
    }

    fn view(&self) -> Element<'_, Message> {
        let status = match &self.shell {
            Err(error) => format!("No shell could be started: {error}"),
            Ok(_) if self.ended => "The shell has ended.".to_owned(),
            Ok(_) => format!(
                "{} x {}{}",
                self.content.cols(),
                self.content.rows(),
                self.content
                    .title()
                    .map(|title| format!("  {title}"))
                    .unwrap_or_default()
            ),
        };
        let typing = self.shell.is_ok() && !self.ended;
        let status = match &self.link {
            Some(link) => format!("{status}  Link: {link}"),
            None => status,
        };

        let search = self.searching.then(|| {
            let found = match (
                self.content.search_position(),
                self.content.search_count(),
            ) {
                _ if self.content.search_needle().is_empty() => String::new(),
                (Some(position), count) => format!("{position} of {count}"),
                (None, _) => "Not found".to_owned(),
            };
            let some = self.content.search_count() > 0;

            row![
                text_input(
                    "Find in the terminal",
                    self.content.search_needle()
                )
                .id(SEARCH)
                .on_input(Message::Search)
                .on_submit(Message::SearchPrevious)
                .size(13)
                .width(260),
                text(found).size(13).width(Fill),
                button(text("Previous").size(13))
                    .padding([2, 8])
                    .on_press_maybe(some.then_some(Message::SearchPrevious)),
                button(text("Next").size(13))
                    .padding([2, 8])
                    .on_press_maybe(some.then_some(Message::SearchNext)),
                button(text("Close").size(13))
                    .padding([2, 8])
                    .style(button::secondary)
                    .on_press(Message::SearchToggled),
            ]
            .spacing(8)
            .align_y(Center)
        });

        column![
            terminal(&self.content)
                .on_input_maybe(typing.then_some(Message::Typed))
                .on_fit(Message::Fit)
                .on_link(Message::Link)
                .on_search(Message::SearchToggled),
            search,
            container(
                row![
                    text(status).size(13),
                    text("Shift+Esc: stop typing. Ctrl+Shift+C / V / F: copy, paste, find. Ctrl+click: a link.")
                        .size(13)
                        .width(Fill)
                        .align_x(iced::Right),
                ]
                .spacing(16)
            )
            .padding([4, 2]),
        ]
        .padding(8)
        .spacing(4)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced::advanced::clipboard::Kind;
    use iced::keyboard::key::{self, Named};
    use iced::keyboard::{self, Modifiers};
    use iced::mouse;
    use iced::{Event, Point, Rectangle};
    use iced_test::selector::Text;
    use iced_test::{Simulator, simulator};

    /// The example as it is with a shell, without one: these tests feed
    /// the terminal themselves.
    fn example(output: &[u8]) -> Example {
        let mut example = Example {
            content: terminal::Content::new(80, 24),
            shell: Err("no shell in a test".to_owned()),
            inbox: Inbox::new(),
            ended: false,
            searching: false,
            link: None,
        };

        let _ = example.update(Message::Output(output.to_vec()));
        example
    }

    /// A view that takes input, as with a shell.
    fn view(example: &Example) -> Element<'_, Message> {
        terminal(&example.content)
            .on_input(Message::Typed)
            .on_fit(Message::Fit)
            .on_link(Message::Link)
            .on_search(Message::SearchToggled)
            .into()
    }

    fn numbered(lines: usize) -> Vec<u8> {
        let mut output = Vec::new();

        for n in 1..=lines {
            output.extend_from_slice(format!("line {n}\r\n").as_bytes());
        }

        output
    }

    fn move_to(ui: &mut Simulator<'_, Message>, to: Point) {
        ui.point_at(to);
        let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved {
            position: to,
        })]);
    }

    fn hold(ui: &mut Simulator<'_, Message>, modifiers: Modifiers) {
        let _ = ui.simulate([Event::Keyboard(
            keyboard::Event::ModifiersChanged(modifiers),
        )]);
    }

    fn links(ui: Simulator<'_, Message>) -> Vec<String> {
        ui.into_messages()
            .filter_map(|message| match message {
                Message::Link(url) => Some(url),
                _ => None,
            })
            .collect()
    }

    fn bounds(ui: &mut Simulator<'_, Message>, content: &str) -> Rectangle {
        match ui.find(content).expect("the line is on the screen") {
            Text::Raw { bounds, .. } | Text::Input { bounds, .. } => bounds,
        }
    }

    fn press(ui: &mut Simulator<'_, Message>, at: Point) {
        ui.point_at(at);
        let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
            mouse::Button::Left,
        ))]);
    }

    fn release(ui: &mut Simulator<'_, Message>) {
        let _ = ui.simulate([Event::Mouse(mouse::Event::ButtonReleased(
            mouse::Button::Left,
        ))]);
    }

    fn drag(ui: &mut Simulator<'_, Message>, from: Point, to: Point) {
        press(ui, from);
        ui.point_at(to);
        let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved {
            position: to,
        })]);
        release(ui);
    }

    fn key(
        ui: &mut Simulator<'_, Message>,
        key: keyboard::Key,
        modifiers: Modifiers,
    ) {
        let text = match &key {
            keyboard::Key::Character(c) if modifiers.is_empty() => {
                Some(c.clone())
            }
            _ => None,
        };

        let _ = ui.simulate([Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Unidentified(
                key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text,
            repeat: false,
        })]);
    }

    fn typed(ui: Simulator<'_, Message>) -> Vec<u8> {
        ui.into_messages()
            .filter_map(|message| match message {
                Message::Typed(bytes) => Some(bytes),
                _ => None,
            })
            .flatten()
            .collect()
    }

    #[test]
    fn what_is_fed_is_shown_line_by_line() {
        let example =
            example(b"$ make test\r\n\x1b[32mok\x1b[0m 12 passed\r\n$ ");
        let mut ui = simulator(view(&example));

        assert!(ui.find("$ make test").is_ok());
        assert!(ui.find("ok 12 passed").is_ok());
        assert!(ui.find("12 failed").is_err());
    }

    #[test]
    fn a_click_starts_typing_and_keys_become_the_bytes_of_a_terminal() {
        let example = example(b"$ ");
        let mut ui = simulator(view(&example));

        let line = bounds(&mut ui, "$");
        press(&mut ui, line.center());
        release(&mut ui);

        key(
            &mut ui,
            keyboard::Key::Character("l".into()),
            Modifiers::empty(),
        );
        key(
            &mut ui,
            keyboard::Key::Character("s".into()),
            Modifiers::empty(),
        );
        key(
            &mut ui,
            keyboard::Key::Named(Named::Enter),
            Modifiers::empty(),
        );
        key(
            &mut ui,
            keyboard::Key::Named(Named::ArrowUp),
            Modifiers::empty(),
        );
        key(
            &mut ui,
            keyboard::Key::Character("c".into()),
            Modifiers::CTRL,
        );
        key(
            &mut ui,
            keyboard::Key::Named(Named::Tab),
            Modifiers::empty(),
        );

        assert_eq!(typed(ui), b"ls\r\x1b[A\x03\t");
    }

    #[test]
    fn the_arrows_follow_the_mode_the_program_asked_for() {
        // A full screen program asks for "application" arrows
        let example = example(b"\x1b[?1h");
        let mut ui = simulator(view(&example));

        press(&mut ui, Point::new(100.0, 100.0));
        release(&mut ui);
        key(
            &mut ui,
            keyboard::Key::Named(Named::ArrowUp),
            Modifiers::empty(),
        );

        assert_eq!(typed(ui), b"\x1bOA");
    }

    #[test]
    fn tab_reaches_it_enter_starts_typing_and_shift_escape_stops() {
        let example = example(b"$ ");
        let mut ui = simulator(view(&example));

        // Reached with Tab it has the focus and does not take the keys yet
        let _ = ui.tap_key(Named::Tab);
        key(
            &mut ui,
            keyboard::Key::Character("x".into()),
            Modifiers::empty(),
        );

        // Enter starts typing; it is not sent itself
        key(
            &mut ui,
            keyboard::Key::Named(Named::Enter),
            Modifiers::empty(),
        );
        key(
            &mut ui,
            keyboard::Key::Character("y".into()),
            Modifiers::empty(),
        );

        // Shift+Escape stops; what follows is not sent
        key(
            &mut ui,
            keyboard::Key::Named(Named::Escape),
            Modifiers::SHIFT,
        );
        key(
            &mut ui,
            keyboard::Key::Character("z".into()),
            Modifiers::empty(),
        );

        assert_eq!(typed(ui), b"y");
    }

    #[test]
    fn a_terminal_without_input_types_nothing() {
        let example = example(b"a recording\r\n");
        let mut ui =
            simulator(Element::from(terminal::<Message, _>(&example.content)));

        let line = bounds(&mut ui, "a recording");
        press(&mut ui, line.center());
        release(&mut ui);
        key(
            &mut ui,
            keyboard::Key::Character("x".into()),
            Modifiers::empty(),
        );
        key(
            &mut ui,
            keyboard::Key::Named(Named::Enter),
            Modifiers::empty(),
        );

        assert_eq!(ui.into_messages().count(), 0);
    }

    #[test]
    fn a_drag_selects_cells_and_the_copy_keys_copy() {
        let example = example(b"first line\r\nsecond line\r\n");
        let mut ui = simulator(view(&example));

        let first = bounds(&mut ui, "first line");
        let second = bounds(&mut ui, "second line");

        // From the start of the first line to the end of the second
        drag(
            &mut ui,
            Point::new(first.x + 1.0, first.center_y()),
            Point::new(second.x + second.width - 1.0, second.center_y()),
        );

        assert_eq!(
            ui.clipboard(Kind::Primary),
            Some("first line\nsecond line"),
            "the selection is the primary selection"
        );
        assert_eq!(ui.clipboard(Kind::Standard), None, "not copied yet");

        key(
            &mut ui,
            keyboard::Key::Character("C".into()),
            if cfg!(target_os = "macos") {
                Modifiers::LOGO
            } else {
                Modifiers::CTRL | Modifiers::SHIFT
            },
        );
        assert_eq!(
            ui.clipboard(Kind::Standard),
            Some("first line\nsecond line")
        );

        // Copying is not typing
        assert_eq!(typed(ui), b"");
    }

    #[test]
    fn a_double_click_selects_a_word_and_a_triple_click_the_line() {
        let example = example(b"cargo test --workspace\r\n");
        let mut ui = simulator(view(&example));

        let line = bounds(&mut ui, "cargo test --workspace");
        // Inside "cargo"
        let at = Point::new(line.x + 12.0, line.center_y());

        press(&mut ui, at);
        release(&mut ui);
        press(&mut ui, at);
        release(&mut ui);
        assert_eq!(ui.clipboard(Kind::Primary), Some("cargo"));

        press(&mut ui, at);
        release(&mut ui);
        assert_eq!(ui.clipboard(Kind::Primary), Some("cargo test --workspace"));
    }

    #[test]
    fn pasting_types_the_clipboard() {
        let example = example(b"\x1b[?2004h$ ");
        let mut ui = simulator(view(&example));

        // Something to paste: copied from the terminal itself
        let line = bounds(&mut ui, "$");
        press(&mut ui, Point::new(line.x + 1.0, line.center_y()));
        release(&mut ui);
        press(&mut ui, Point::new(line.x + 1.0, line.center_y()));
        release(&mut ui);
        assert_eq!(ui.clipboard(Kind::Primary), Some("$"));

        let chord = if cfg!(target_os = "macos") {
            Modifiers::LOGO
        } else {
            Modifiers::CTRL | Modifiers::SHIFT
        };
        key(&mut ui, keyboard::Key::Character("C".into()), chord);
        key(&mut ui, keyboard::Key::Character("V".into()), chord);

        // The program asked for the paste marks
        assert_eq!(typed(ui), b"\x1b[200~$\x1b[201~");
    }

    #[test]
    fn the_wheel_scrolls_through_what_scrolled_away() {
        let example = example(&numbered(200));
        let mut ui = simulator(view(&example));

        assert!(ui.find("line 200").is_ok());
        assert!(ui.find("line 100").is_err(), "far above the window");

        ui.point_at(Point::new(200.0, 200.0));
        let _ = ui.simulate([Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: 30.0 },
        })]);

        assert!(ui.find("line 200").is_err(), "scrolled up");
        assert!(ui.find("line 100").is_ok());
    }

    #[test]
    fn a_search_marks_every_place_and_scrolls_to_the_current_one()
    -> Result<(), iced_test::Error> {
        let mut example = example(&numbered(200));

        {
            let mut ui = simulator(view(&example));
            assert!(ui.find("line 42").is_err(), "far above the window");
        }

        // "line 4" is in line 4 and in 40 to 49
        let _ = example.update(Message::Search("line 4".to_owned()));
        assert_eq!(example.content.search_count(), 1 + 10);
        assert_eq!(
            example.content.search_position(),
            Some(11),
            "the newest place is the current one"
        );

        {
            let mut ui = simulator(view(&example));
            assert!(ui.find("line 49").is_ok(), "scrolled to it");
            assert!(ui.find("line 200").is_err());
        }

        // Before the first comes the last, and back
        let _ = example.update(Message::SearchNext);
        assert_eq!(example.content.search_position(), Some(1));

        {
            let mut ui = simulator(view(&example));
            assert!(ui.find("line 4").is_ok());
            assert!(ui.find("line 49").is_err());
        }

        let _ = example.update(Message::SearchPrevious);
        let _ = example.update(Message::SearchPrevious);
        assert_eq!(example.content.search_position(), Some(10));

        // A capital in what is searched for makes capitals count
        let _ = example.update(Message::Search("Line 4".to_owned()));
        assert_eq!(example.content.search_count(), 0);
        assert_eq!(example.content.search_position(), None);

        // What arrives while the search is on is searched too, and the
        // current place stays the current one
        let _ = example.update(Message::Search("line 20".to_owned()));
        assert_eq!(example.content.search_count(), 2);
        let _ = example.update(Message::SearchPrevious);
        assert_eq!(example.content.search_position(), Some(1));
        let _ = example.update(Message::Output(b"line 2000\r\n".to_vec()));
        assert_eq!(example.content.search_count(), 3);
        assert_eq!(example.content.search_position(), Some(1));

        // The places are drawn, the current one unlike the others: the
        // same lines with a place marked, with another one current, and
        // with none
        let _ = example.update(Message::Search("line 19".to_owned()));
        let picture = |example: &Example| {
            let mut ui = simulator(view(example));
            assert!(
                ui.find("line 199").is_ok() && ui.find("line 2000").is_ok()
            );
            ui.snapshot(&iced::Theme::Dark).map(|s| s.into_rgba().0)
        };
        let last_current = picture(&example)?;

        let _ = example.update(Message::SearchPrevious);
        assert!(example.content.search_position() < Some(11));
        let another_current = picture(&example)?;

        let _ = example.update(Message::Search(String::new()));
        assert_eq!(example.content.search_count(), 0);
        assert_eq!(example.content.search_needle(), "");
        let plain = picture(&example)?;

        assert_ne!(last_current, plain);
        assert_ne!(last_current, another_current);
        assert_ne!(another_current, plain);

        Ok(())
    }

    #[test]
    fn the_key_of_find_shows_the_search_field_of_the_example() {
        let mut example = example(b"$ ");

        let toggled = {
            let mut ui = simulator(view(&example));

            press(&mut ui, Point::new(100.0, 100.0));
            release(&mut ui);
            key(
                &mut ui,
                keyboard::Key::Character("F".into()),
                if cfg!(target_os = "macos") {
                    Modifiers::LOGO
                } else {
                    Modifiers::CTRL | Modifiers::SHIFT
                },
            );

            let messages: Vec<Message> = ui.into_messages().collect();

            assert!(
                !messages.iter().any(|m| matches!(m, Message::Typed(_))),
                "the key is not typed into the program"
            );
            messages
                .iter()
                .filter(|m| matches!(m, Message::SearchToggled))
                .count()
        };
        assert_eq!(toggled, 1);

        let _ = example.update(Message::SearchToggled);

        {
            let mut ui = simulator(example.view());
            assert!(ui.find("Previous").is_ok());
            let _ = ui.click("Close").expect("the field has a way out");
            assert!(
                ui.into_messages()
                    .any(|m| matches!(m, Message::SearchToggled))
            );
        }

        let _ = example.update(Message::Search("$".to_owned()));
        let _ = example.update(Message::SearchToggled);
        assert_eq!(example.content.search_count(), 0, "closing ends it");

        let mut ui = simulator(example.view());
        assert!(ui.find("Previous").is_err());
    }

    #[test]
    fn a_drag_past_the_top_scrolls_for_as_long_as_it_is_held_there() {
        use iced::time::{Duration, Instant};
        use iced::window;

        let example = example(&numbered(200));
        let mut ui = simulator(view(&example));

        let start = bounds(&mut ui, "line 190");
        press(&mut ui, Point::new(start.x + 1.0, start.center_y()));

        // Above the terminal, and resting there
        move_to(&mut ui, Point::new(start.x + 1.0, -8.0));
        assert!(ui.find("line 100").is_err());

        let begun = Instant::now() + Duration::from_millis(60);

        for frame in 0..90 {
            let _ =
                ui.simulate([Event::Window(window::Event::RedrawRequested(
                    begun + Duration::from_millis(60) * frame,
                ))]);
        }

        assert!(
            ui.find("line 100").is_ok(),
            "the view went up while the pointer did not move"
        );
        assert!(ui.find("line 190").is_err());

        // Frames that come sooner than the step do not scroll
        let top = |ui: &mut Simulator<'_, Message>| {
            (1..=200)
                .find(|n| ui.find(format!("line {n}")).is_ok())
                .expect("a line is shown")
        };
        let before = top(&mut ui);
        let later = begun + Duration::from_millis(60) * 89;

        for _ in 0..20 {
            let _ = ui.simulate([Event::Window(
                window::Event::RedrawRequested(later),
            )]);
        }

        assert_eq!(top(&mut ui), before);

        release(&mut ui);

        let selected = ui.clipboard(Kind::Primary).expect("a selection");
        assert!(selected.ends_with("line 189\nl"), "{selected:?}");
        assert!(
            selected.lines().count() > 60,
            "from where the view went to: {} lines",
            selected.lines().count()
        );

        // Once released nothing follows any more
        let _ = ui.simulate([Event::Window(window::Event::RedrawRequested(
            later + Duration::from_secs(1),
        ))]);
        assert_eq!(top(&mut ui), before);
    }

    #[test]
    fn a_program_that_asks_is_told_of_the_mouse() {
        // Presses and releases, in digits (what vim, htop and tmux ask for)
        let example = example(b"one\r\ntwo\r\nthree\x1b[?1000h\x1b[?1006h");
        let mut ui = simulator(view(&example));

        let three = bounds(&mut ui, "three");
        let two = bounds(&mut ui, "two");
        let at = Point::new(three.x + 1.0, three.center_y());

        press(&mut ui, at);
        // In this mode a drag is not told
        move_to(&mut ui, Point::new(two.x + 1.0, two.center_y()));
        move_to(&mut ui, at);
        release(&mut ui);

        let _ = ui.simulate([Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: 1.0 },
        })]);
        let _ = ui.simulate([Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0.0, y: -2.0 },
        })]);

        ui.point_at(at);
        let _ = ui.simulate([
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Right)),
        ]);

        assert_eq!(ui.clipboard(Kind::Primary), None, "nothing is selected");
        assert_eq!(
            String::from_utf8(typed(ui)).unwrap(),
            "\x1b[<0;1;3M\x1b[<0;1;3m\
             \x1b[<64;1;3M\x1b[<65;1;3M\x1b[<65;1;3M\
             \x1b[<2;1;3M\x1b[<2;1;3m"
        );
    }

    #[test]
    fn a_drag_is_told_when_asked_and_the_old_encoding_is_three_bytes() {
        let example = example(b"one\r\ntwo\r\nthree\x1b[?1002h");
        let mut ui = simulator(view(&example));

        let one = bounds(&mut ui, "one");
        let two = bounds(&mut ui, "two");

        // Without a button nothing is told in this mode
        move_to(&mut ui, Point::new(one.x + 1.0, one.center_y()));
        press(&mut ui, Point::new(one.x + 1.0, one.center_y()));
        move_to(&mut ui, Point::new(two.x + 1.0, two.center_y()));
        // The same cell again says nothing
        move_to(&mut ui, Point::new(two.x + 2.0, two.center_y()));
        release(&mut ui);

        assert_eq!(
            typed(ui),
            b"\x1b[M\x20\x21\x21\x1b[M\x40\x21\x22\x1b[M\x23\x21\x22"
        );
    }

    #[test]
    fn every_movement_is_told_to_a_program_that_asks_for_it() {
        let example = example(b"one\r\ntwo\r\n\x1b[?1003h\x1b[?1006h");
        let mut ui = simulator(view(&example));

        let one = bounds(&mut ui, "one");
        let two = bounds(&mut ui, "two");

        move_to(&mut ui, Point::new(one.x + 1.0, one.center_y()));
        move_to(&mut ui, Point::new(two.x + 1.0, two.center_y()));

        assert_eq!(typed(ui), b"\x1b[<35;1;1M\x1b[<35;1;2M");
    }

    #[test]
    fn shift_gives_the_mouse_back_to_the_selection() {
        let example = example(b"first line\r\n\x1b[?1000h\x1b[?1006h");
        let mut ui = simulator(view(&example));

        let first = bounds(&mut ui, "first line");

        hold(&mut ui, Modifiers::SHIFT);
        drag(
            &mut ui,
            Point::new(first.x + 1.0, first.center_y()),
            Point::new(first.x + first.width - 1.0, first.center_y()),
        );

        assert_eq!(ui.clipboard(Kind::Primary), Some("first line"));
        assert_eq!(typed(ui), b"", "the program is told nothing");
    }

    #[test]
    fn a_terminal_that_only_shows_tells_no_program_of_the_mouse() {
        let example = example(b"a recording\r\n\x1b[?1003h\x1b[?1006h");
        let mut ui =
            simulator(Element::from(terminal::<Message, _>(&example.content)));

        let line = bounds(&mut ui, "a recording");
        drag(
            &mut ui,
            Point::new(line.x + 1.0, line.center_y()),
            Point::new(line.x + line.width - 1.0, line.center_y()),
        );

        assert_eq!(ui.clipboard(Kind::Primary), Some("a recording"));
        assert_eq!(ui.into_messages().count(), 0);
    }

    #[test]
    fn a_link_is_clicked_with_the_command_key_held() {
        let example =
            example(b"https://example.com/docs is the manual\r\nno link\r\n");

        // A plain click is a click into the terminal
        {
            let mut ui = simulator(view(&example));
            let line =
                bounds(&mut ui, "https://example.com/docs is the manual");
            let at = Point::new(line.x + 20.0, line.center_y());

            move_to(&mut ui, at);
            press(&mut ui, at);
            release(&mut ui);
            assert_eq!(links(ui), [] as [String; 0]);
        }

        let mut ui = simulator(view(&example));
        let line = bounds(&mut ui, "https://example.com/docs is the manual");
        let other = bounds(&mut ui, "no link");
        let at = Point::new(line.x + 20.0, line.center_y());

        hold(
            &mut ui,
            if cfg!(target_os = "macos") {
                Modifiers::LOGO
            } else {
                Modifiers::CTRL
            },
        );
        move_to(&mut ui, at);
        press(&mut ui, at);
        release(&mut ui);

        // Where no link is, the key changes nothing
        press(&mut ui, Point::new(other.x + 20.0, other.center_y()));
        release(&mut ui);

        assert_eq!(ui.clipboard(Kind::Primary), None, "it selects nothing");
        assert_eq!(links(ui), ["https://example.com/docs"]);
    }

    #[test]
    fn a_plain_click_opens_a_link_where_that_is_asked_for() {
        let example =
            example(b"https://example.com/docs is the manual\r\nno link\r\n");
        let view = || -> Element<'_, Message> {
            terminal(&example.content)
                .on_link(Message::Link)
                .plain_link_click(true)
                .into()
        };

        let mut ui = simulator(view());
        let line = bounds(&mut ui, "https://example.com/docs is the manual");
        let at = Point::new(line.x + 20.0, line.center_y());

        press(&mut ui, at);
        release(&mut ui);
        assert_eq!(links(ui), ["https://example.com/docs"]);

        // A drag that began on it selects and opens nothing
        let mut ui = simulator(view());
        drag(
            &mut ui,
            at,
            Point::new(line.x + line.width - 1.0, line.center_y()),
        );
        assert!(ui.clipboard(Kind::Primary).is_some());
        assert_eq!(links(ui), [] as [String; 0]);
    }

    #[test]
    fn the_link_under_the_pointer_is_underlined() -> Result<(), iced_test::Error>
    {
        let example = example(b"https://example.com/docs is the manual\r\n");
        let mut ui = simulator(view(&example));
        let line = bounds(&mut ui, "https://example.com/docs is the manual");

        move_to(&mut ui, Point::new(line.x + 20.0, line.center_y()));
        let over = ui.snapshot(&iced::Theme::Dark)?.into_rgba().0;

        move_to(&mut ui, Point::new(line.x + 20.0, line.center_y() + 200.0));
        let away = ui.snapshot(&iced::Theme::Dark)?.into_rgba().0;

        assert_ne!(over, away);
        Ok(())
    }

    #[test]
    fn it_says_how_many_cells_fit() {
        let example = example(b"");
        let mut ui = simulator(view(&example));

        ui.point_at(Point::new(10.0, 10.0));
        let _ = ui.simulate([Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(10.0, 10.0),
        })]);

        let fits: Vec<(u16, u16)> = ui
            .into_messages()
            .filter_map(|message| match message {
                Message::Fit(cols, rows) => Some((cols, rows)),
                _ => None,
            })
            .collect();

        assert_eq!(fits.len(), 1, "said once");
        // 1024 x 768 at the default size: more than a few and not absurd
        assert!((60..300).contains(&fits[0].0), "{fits:?}");
        assert!((20..100).contains(&fits[0].1), "{fits:?}");
    }

    #[test]
    fn colors_the_cursor_and_the_selection_are_drawn()
    -> Result<(), iced_test::Error> {
        let plain = example(b"status ok\r\n");
        let colored = example(b"status \x1b[1;32mok\x1b[0m\r\n");
        let on_blue = example(b"status \x1b[44mok\x1b[0m\r\n");

        let picture = |example: &Example| {
            let mut ui = simulator(view(example));
            ui.snapshot(&iced::Theme::Dark).map(|s| s.into_rgba().0)
        };

        let plain_picture = picture(&plain)?;
        assert_ne!(plain_picture, picture(&colored)?, "a color and bold");
        assert_ne!(plain_picture, picture(&on_blue)?, "a background");

        // The selection
        let mut ui = simulator(view(&plain));
        let line = bounds(&mut ui, "status ok");
        drag(
            &mut ui,
            Point::new(line.x + 1.0, line.center_y()),
            Point::new(line.x + 60.0, line.center_y()),
        );
        let selected = ui.snapshot(&iced::Theme::Dark)?.into_rgba().0;
        assert_ne!(plain_picture, selected);

        // Light and dark differ
        let mut ui = simulator(view(&plain));
        let light = ui.snapshot(&iced::Theme::Light)?.into_rgba().0;
        assert_ne!(plain_picture, light);

        Ok(())
    }
}
