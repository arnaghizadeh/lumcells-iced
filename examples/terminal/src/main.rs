use iced::subscription::Inbox;
use iced::widget::{column, container, row, terminal, text};
use iced::{Element, Fill, Subscription};

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
}

#[derive(Debug, Clone)]
enum Message {
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
        }
    }

    fn update(&mut self, message: Message) {
        match message {
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

        column![
            terminal(&self.content)
                .on_input_maybe(typing.then_some(Message::Typed))
                .on_fit(Message::Fit),
            container(
                row![
                    text(status).size(13),
                    text("Enter or a click: type. Shift+Esc: stop typing. Ctrl+Shift+C / V: copy, paste.")
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
        };

        example.update(Message::Output(output.to_vec()));
        example
    }

    /// A view that takes input, as with a shell.
    fn view(example: &Example) -> Element<'_, Message> {
        terminal(&example.content)
            .on_input(Message::Typed)
            .on_fit(Message::Fit)
            .into()
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
        let mut output = Vec::new();

        for n in 1..=200 {
            output.extend_from_slice(format!("line {n}\r\n").as_bytes());
        }

        let example = example(&output);
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
