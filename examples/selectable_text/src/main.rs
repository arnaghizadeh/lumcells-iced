use iced::widget::{
    center, column, container, markdown, rich_text, rule, selection_area, span,
    text, text_input,
};
use iced::{Element, Fill, Font, Theme, font};

pub fn main() -> iced::Result {
    iced::application(Example::default, Example::update, Example::view)
        .title("Selectable text")
        .window_size((560.0, 760.0))
        .run()
}

const ERROR: &str =
    "The server cannot be reached: connection refused (os error 111)";
const PATH: &str = "/home/ada/Lumcells Drive/Reports/2026-11 budget.ods";
const PARAGRAPH: &str = "This paragraph is long enough to be wrapped over a few lines, so that a selection that begins on one line and ends on another can be seen. Select a part of it and press Ctrl+C.";

// A report in three texts: a selection goes from one to the next
const REPORT_TITLE: &str = "Report of 23 November";
const REPORT_FIRST: &str = "Three files could not be synced.";
const REPORT_PLACE: &str = "/home/ada/.local/state/drive/report.txt";

const NOTES: &str = "## Release notes

The **tray** icon has a *name* now, see [the notes](https://example.com/notes).

- Focus goes round in a dialog
- Text can be selected
";

struct Example {
    pasted: String,
    notes: markdown::Content,
    opened: Option<markdown::Uri>,
}

impl Default for Example {
    fn default() -> Self {
        Self {
            pasted: String::new(),
            notes: markdown::Content::parse(NOTES),
            opened: None,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Pasted(String),
    LinkClicked(markdown::Uri),
}

impl Example {
    fn update(&mut self, message: Message) {
        match message {
            Message::Pasted(pasted) => self.pasted = pasted,
            Message::LinkClicked(uri) => self.opened = Some(uri),
        }
    }

    /// The text of the rich text below, as it is copied.
    #[cfg(test)]
    fn rich() -> String {
        format!("The file {REPORT_PLACE} holds what was sent.")
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
                rule::horizontal(1),
                // Several texts as one: a drag goes from one to the next,
                // Ctrl+A after a click inside selects all of them, and
                // the copy key copies them with a line feed between
                selection_area(
                    column![
                        text(REPORT_TITLE).size(18).selectable(true),
                        text(REPORT_FIRST).selectable(true),
                        // Spans of several styles are one text to select
                        rich_text![
                            span("The file "),
                            span(REPORT_PLACE).font(Font::MONOSPACE),
                            span(" holds "),
                            span("what was sent").font(Font {
                                weight: font::Weight::Bold,
                                ..Font::DEFAULT
                            }),
                            span("."),
                        ]
                        .on_link_click(std::convert::identity::<Message>)
                        .selectable(true),
                    ]
                    .spacing(6)
                ),
                rule::horizontal(1),
                // Markdown that can be selected is such an area itself
                markdown::view(
                    self.notes.items(),
                    markdown::Settings::with_style(markdown::Style::from(
                        &Theme::Light
                    ))
                    .selectable(true),
                )
                .map(Message::LinkClicked),
                text(match &self.opened {
                    Some(uri) => format!("A click on the link: {uri}"),
                    None => "The link can still be clicked.".to_owned(),
                }),
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

    fn click(ui: &mut Simulator<'_, Message>, at: Point) {
        press(ui, at);
        release(ui);
    }

    fn tap(
        ui: &mut Simulator<'_, Message>,
        key: keyboard::Key,
        code: key::Code,
        modifiers: Modifiers,
    ) {
        let _ = ui.simulate([Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Code(code),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })]);
    }

    fn shift(
        ui: &mut Simulator<'_, Message>,
        named: key::Named,
        more: Modifiers,
    ) {
        tap(
            ui,
            keyboard::Key::Named(named),
            key::Code::ShiftLeft,
            Modifiers::SHIFT | more,
        );
    }

    fn select_all(ui: &mut Simulator<'_, Message>) {
        tap(
            ui,
            keyboard::Key::Character("a".into()),
            key::Code::KeyA,
            Modifiers::COMMAND,
        );
    }

    fn primary(ui: &Simulator<'_, Message>) -> String {
        ui.clipboard(Kind::Primary).unwrap_or_default().to_owned()
    }

    #[test]
    fn shift_and_the_arrows_move_the_end_of_the_selection() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let line = bounds(&mut ui, ERROR);
        click(&mut ui, Point::new(line.x, line.center_y()));

        for _ in 0..3 {
            shift(&mut ui, key::Named::ArrowRight, Modifiers::empty());
        }
        assert_eq!(primary(&ui), "The");

        shift(&mut ui, key::Named::ArrowLeft, Modifiers::empty());
        assert_eq!(primary(&ui), "Th");

        // By words
        let jump = if cfg!(target_os = "macos") {
            Modifiers::ALT
        } else {
            Modifiers::CTRL
        };

        shift(&mut ui, key::Named::ArrowRight, jump);
        assert_eq!(primary(&ui), "The");
        shift(&mut ui, key::Named::ArrowRight, jump);
        assert_eq!(primary(&ui), "The server");
        shift(&mut ui, key::Named::ArrowLeft, jump);
        assert_eq!(primary(&ui), "The ");

        // To the end of the line and back to its start
        shift(&mut ui, key::Named::End, Modifiers::empty());
        assert_eq!(primary(&ui), ERROR);

        shift(&mut ui, key::Named::Home, Modifiers::empty());
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), None, "nothing is selected");

        // Without Shift the arrows are not for the text
        tap(
            &mut ui,
            keyboard::Key::Named(key::Named::ArrowRight),
            key::Code::ArrowRight,
            Modifiers::empty(),
        );
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), None);
    }

    #[test]
    fn shift_with_up_and_down_goes_by_the_lines_as_they_are_wrapped() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let paragraph = bounds(&mut ui, PARAGRAPH);
        click(&mut ui, Point::new(paragraph.x, paragraph.y + 4.0));

        shift(&mut ui, key::Named::ArrowDown, Modifiers::empty());
        let one = primary(&ui);
        assert!(!one.is_empty() && one.len() < PARAGRAPH.len(), "{one:?}");
        assert!(PARAGRAPH.starts_with(&one));

        // The end of the line the caret is on now
        shift(&mut ui, key::Named::End, Modifiers::empty());
        let two = primary(&ui);
        assert!(two.len() > one.len() && two.len() < PARAGRAPH.len());
        assert!(PARAGRAPH.starts_with(&two));

        shift(&mut ui, key::Named::ArrowUp, Modifiers::empty());
        let back = primary(&ui);
        assert!(back.len() < two.len() && PARAGRAPH.starts_with(&back));

        // Down past the last line: everything
        for _ in 0..6 {
            shift(&mut ui, key::Named::ArrowDown, Modifiers::empty());
        }
        assert_eq!(primary(&ui), PARAGRAPH);

        // Up past the first line: nothing
        for _ in 0..6 {
            shift(&mut ui, key::Named::ArrowUp, Modifiers::empty());
        }
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), None);
    }

    #[test]
    fn select_all_is_for_the_text_that_was_clicked() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        // No text was clicked: the key is nobody's
        select_all(&mut ui);
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), None);

        let path = bounds(&mut ui, PATH);
        click(&mut ui, path.center());
        select_all(&mut ui);
        copy(&mut ui);

        assert_eq!(ui.clipboard(Kind::Standard), Some(PATH));
    }

    #[test]
    fn rich_text_is_selected_over_its_spans() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let rich = bounds(&mut ui, &Example::rich());

        drag(
            &mut ui,
            Point::new(rich.x, rich.y + 4.0),
            Point::new(rich.x + rich.width + 40.0, rich.y + rich.height - 4.0),
        );
        copy(&mut ui);

        assert_eq!(
            ui.clipboard(Kind::Standard),
            Some(Example::rich().as_str()),
            "the words of every span, without their styles"
        );

        // A word inside one span
        let mut ui = simulator(example.view());
        click(&mut ui, Point::new(rich.x + 4.0, rich.y + 4.0));
        click(&mut ui, Point::new(rich.x + 4.0, rich.y + 4.0));
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), Some("The"));
    }

    #[test]
    fn a_selection_goes_from_one_text_to_the_next_in_a_selection_area() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let title = bounds(&mut ui, REPORT_TITLE);
        let rich = bounds(&mut ui, &Example::rich());

        // From the middle of the title to the middle of the rich text's
        // first line
        drag(
            &mut ui,
            Point::new(title.x + title.width / 2.0, title.center_y()),
            Point::new(rich.x + rich.width / 2.0, rich.y + 4.0),
        );

        let selected = primary(&ui);
        copy(&mut ui);
        assert_eq!(ui.clipboard(Kind::Standard), Some(selected.as_str()));

        let parts: Vec<_> = selected.split('\n').collect();
        assert_eq!(parts.len(), 3, "{selected:?}");

        assert!(REPORT_TITLE.ends_with(parts[0]) && !parts[0].is_empty());
        assert!(parts[0].len() < REPORT_TITLE.len());
        assert_eq!(parts[1], REPORT_FIRST, "all of what is between");
        assert!(Example::rich().starts_with(parts[2]) && !parts[2].is_empty());
        assert!(parts[2].len() < Example::rich().len());

        // The same drag the other way selects the same
        let mut ui = simulator(example.view());
        drag(
            &mut ui,
            Point::new(rich.x + rich.width / 2.0, rich.y + 4.0),
            Point::new(title.x + title.width / 2.0, title.center_y()),
        );
        assert_eq!(primary(&ui), selected);

        // A click elsewhere drops all of it
        let other = bounds(&mut ui, "This one cannot.");
        click(&mut ui, other.center());
        let _ = ui.simulate([Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Character("c".into()),
            modified_key: keyboard::Key::Character("c".into()),
            physical_key: key::Physical::Code(key::Code::KeyC),
            location: keyboard::Location::Standard,
            modifiers: Modifiers::COMMAND,
            text: None,
            repeat: false,
        })]);
        assert_eq!(ui.clipboard(Kind::Standard), None);
    }

    #[test]
    fn select_all_in_a_selection_area_selects_every_text_in_it() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let first = bounds(&mut ui, REPORT_FIRST);
        click(&mut ui, first.center());
        select_all(&mut ui);
        copy(&mut ui);

        assert_eq!(
            ui.clipboard(Kind::Standard).map(str::to_owned),
            Some(format!(
                "{REPORT_TITLE}\n{REPORT_FIRST}\n{}",
                Example::rich()
            )),
            "and nothing of the texts outside of it"
        );
    }

    #[test]
    fn markdown_is_selected_from_block_to_block_as_it_is_shown() {
        let example = Example::default();
        let mut ui = simulator(example.view());

        let heading = bounds(&mut ui, "Release notes");
        click(&mut ui, heading.center());
        select_all(&mut ui);
        copy(&mut ui);

        assert_eq!(
            ui.clipboard(Kind::Standard),
            Some(
                "Release notes\n\
                 The tray icon has a name now, see the notes.\n\
                 Focus goes round in a dialog\n\
                 Text can be selected"
            ),
            "without the marks of Markdown"
        );

        // A drag from the heading into the paragraph
        let mut ui = simulator(example.view());
        let paragraph =
            bounds(&mut ui, "The tray icon has a name now, see the notes.");

        drag(
            &mut ui,
            Point::new(heading.x, heading.center_y()),
            Point::new(paragraph.x + 60.0, paragraph.center_y()),
        );

        let selected = primary(&ui);
        let (first, second) = selected.split_once('\n').expect("two blocks");
        assert_eq!(first, "Release notes");
        assert!(second.starts_with("The tray") && second.len() < 30);
    }

    #[test]
    fn a_link_in_selectable_markdown_is_still_clicked() {
        let mut example = Example::default();

        let messages: Vec<_> = {
            let mut ui = simulator(example.view());
            let paragraph =
                bounds(&mut ui, "The tray icon has a name now, see the notes.");

            // "the notes" stands at the end of the line
            click(
                &mut ui,
                Point::new(
                    paragraph.x + paragraph.width - 30.0,
                    paragraph.center_y(),
                ),
            );

            ui.into_messages().collect()
        };

        for message in messages {
            example.update(message);
        }

        assert_eq!(
            example.opened.as_ref().map(ToString::to_string).as_deref(),
            Some("https://example.com/notes")
        );
    }
}
