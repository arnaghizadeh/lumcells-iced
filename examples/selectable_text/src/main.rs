use iced::widget::{center, column, container, text, text_input};
use iced::{Element, Fill};

pub fn main() -> iced::Result {
    iced::application(Example::default, Example::update, Example::view)
        .title("Selectable text")
        .window_size((560.0, 360.0))
        .run()
}

const ERROR: &str =
    "The server cannot be reached: connection refused (os error 111)";
const PATH: &str = "/home/ada/Lumcells Drive/Reports/2026-11 budget.ods";
const PARAGRAPH: &str = "This paragraph is long enough to be wrapped over a few lines, so that a selection that begins on one line and ends on another can be seen. Select a part of it and press Ctrl+C.";

#[derive(Default)]
struct Example {
    pasted: String,
}

#[derive(Debug, Clone)]
enum Message {
    Pasted(String),
}

impl Example {
    fn update(&mut self, message: Message) {
        match message {
            Message::Pasted(pasted) => self.pasted = pasted,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        center(
            column![
                text("These can be selected and copied:").size(18),
                text(ERROR).selectable(true),
                container(
                    text(PATH).font(iced::Font::MONOSPACE).selectable(true)
                )
                .padding(8)
                .style(container::rounded_box),
                text(PARAGRAPH).selectable(true),
                text("This one cannot."),
                text_input("Paste here", &self.pasted)
                    .on_input(Message::Pasted),
            ]
            .spacing(14)
            .width(Fill)
            .max_width(480),
        )
        .padding(20)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced::advanced::clipboard::Kind;
    use iced::keyboard::{self, Modifiers, key};
    use iced::mouse;
    use iced::{Event, Point, Rectangle};
    use iced_test::selector::Text;
    use iced_test::{Simulator, simulator};

    fn bounds(ui: &mut Simulator<'_, Message>, content: &str) -> Rectangle {
        match ui.find(content).expect("the text is on the screen") {
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

    fn copy(ui: &mut Simulator<'_, Message>) {
        let _ = ui.simulate([Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character("c".into()),
            modified_key: keyboard::Key::Character("c".into()),
            physical_key: key::Physical::Code(key::Code::KeyC),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::COMMAND,
            text: None,
            repeat: false,
        })]);
    }

    #[test]
    fn a_drag_over_a_line_selects_it_and_the_copy_key_copies() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let line = bounds(&mut ui, ERROR);
        let middle = line.center_y();

        // From before the first letter to past the last one
        drag(
            &mut ui,
            Point::new(line.x, middle),
            Point::new(line.x + line.width + 50.0, middle),
        );

        assert_eq!(ui.clipboard(Kind::Standard), None, "not copied yet");
        assert_eq!(
            ui.clipboard(Kind::Primary),
            Some(ERROR),
            "the selection is the primary selection"
        );

        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), Some(ERROR));
    }

    #[test]
    fn a_part_of_a_line_can_be_selected() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let line = bounds(&mut ui, ERROR);
        let middle = line.center_y();

        // The first half of the line, more or less
        drag(
            &mut ui,
            Point::new(line.x, middle),
            Point::new(line.x + line.width / 2.0, middle),
        );
        copy(&mut ui);

        let copied = ui.clipboard(Kind::Standard).expect("something is copied");

        assert!(ERROR.starts_with(copied), "{copied:?} begins the sentence");
        assert!(copied.len() > ERROR.len() / 4 && copied.len() < ERROR.len());
    }

    #[test]
    fn a_selection_can_span_wrapped_lines_and_go_backwards() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let paragraph = bounds(&mut ui, PARAGRAPH);
        assert!(paragraph.height > 40.0, "the paragraph is wrapped");

        // From the end of the last line back to the start of the first
        drag(
            &mut ui,
            Point::new(
                paragraph.x + paragraph.width - 1.0,
                paragraph.y + paragraph.height - 2.0,
            ),
            Point::new(paragraph.x, paragraph.y + 2.0),
        );
        copy(&mut ui);

        assert_eq!(ui.clipboard(Kind::Standard), Some(PARAGRAPH));
    }

    #[test]
    fn a_double_click_selects_a_word_and_a_triple_click_everything() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let line = bounds(&mut ui, ERROR);
        // Inside "The", the first word
        let at = Point::new(line.x + 6.0, line.center_y());

        press(&mut ui, at);
        release(&mut ui);
        press(&mut ui, at);
        release(&mut ui);
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), Some("The"));

        press(&mut ui, at);
        release(&mut ui);
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), Some(ERROR));
    }

    #[test]
    fn a_click_elsewhere_drops_the_selection() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let line = bounds(&mut ui, ERROR);
        drag(
            &mut ui,
            Point::new(line.x, line.center_y()),
            Point::new(line.x + line.width, line.center_y()),
        );

        // On the text that is not selectable
        let other = bounds(&mut ui, "This one cannot.");
        press(&mut ui, other.center());
        release(&mut ui);

        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), None, "nothing to copy");
    }

    #[test]
    fn text_that_is_not_selectable_is_not_selected() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let line = bounds(&mut ui, "This one cannot.");
        drag(
            &mut ui,
            Point::new(line.x, line.center_y()),
            Point::new(line.x + line.width, line.center_y()),
        );
        copy(&mut ui);

        assert_eq!(ui.clipboard(Kind::Standard), None);
        assert_eq!(ui.clipboard(Kind::Primary), None);
    }

    #[test]
    fn the_selection_is_drawn() -> Result<(), iced_test::Error> {
        let example = Example::default();
        let mut ui = simulator(example.view());
        let (before, _) = ui.snapshot(&iced::Theme::Light)?.into_rgba();

        let line = bounds(&mut ui, PATH);
        drag(
            &mut ui,
            Point::new(line.x, line.center_y()),
            Point::new(line.x + line.width, line.center_y()),
        );
        let (after, _) = ui.snapshot(&iced::Theme::Light)?.into_rgba();

        assert_ne!(before, after);

        Ok(())
    }
}
