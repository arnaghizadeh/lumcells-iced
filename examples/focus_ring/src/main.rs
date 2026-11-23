use iced::widget::{
    button, center, checkbox, column, pick_list, radio, row, text, text_input,
    toggler,
};
use iced::{Center, Element};

pub fn main() -> iced::Result {
    iced::application(Form::default, Form::update, Form::view)
        .title("Focus ring")
        .window_size((460.0, 440.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Size {
    Small,
    #[default]
    Medium,
    Large,
}

impl Size {
    const ALL: [Size; 3] = [Size::Small, Size::Medium, Size::Large];
}

impl std::fmt::Display for Size {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Size::Small => "Small",
            Size::Medium => "Medium",
            Size::Large => "Large",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Plan {
    #[default]
    Monthly,
    Yearly,
}

#[derive(Debug, Default)]
struct Form {
    name: String,
    agreed: bool,
    notify: bool,
    plan: Plan,
    size: Size,
    saved: u32,
    cancelled: u32,
}

#[derive(Debug, Clone)]
enum Message {
    NameChanged(String),
    Agreed(bool),
    Notify(bool),
    PlanChosen(Plan),
    SizeChosen(Size),
    Save,
    Cancel,
}

impl Form {
    fn update(&mut self, message: Message) {
        match message {
            Message::NameChanged(name) => self.name = name,
            Message::Agreed(agreed) => self.agreed = agreed,
            Message::Notify(notify) => self.notify = notify,
            Message::PlanChosen(plan) => self.plan = plan,
            Message::SizeChosen(size) => self.size = size,
            Message::Save => self.saved += 1,
            Message::Cancel => self.cancelled += 1,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let form = column![
            text("Press Tab to move, Space or Enter to press").size(18),
            text_input("Your name", &self.name)
                .id("name")
                .on_input(Message::NameChanged),
            checkbox(self.agreed)
                .id("agree")
                .label("I agree with the terms")
                .on_toggle(Message::Agreed),
            toggler(self.notify)
                .id("notify")
                .label("Tell me when it is done")
                .on_toggle(Message::Notify),
            row![
                radio(
                    "Monthly",
                    Plan::Monthly,
                    Some(self.plan),
                    Message::PlanChosen
                )
                .id("monthly"),
                radio(
                    "Yearly",
                    Plan::Yearly,
                    Some(self.plan),
                    Message::PlanChosen
                )
                .id("yearly"),
            ]
            .spacing(20),
            pick_list(Size::ALL, Some(self.size), Message::SizeChosen)
                .id("size"),
            row![
                button("Save").id("save").on_press(Message::Save),
                button("Cancel")
                    .id("cancel")
                    .style(button::secondary)
                    .on_press(Message::Cancel),
                // A disabled button is not a stop of the Tab key
                button("Not yet").id("disabled"),
            ]
            .spacing(10),
            text!(
                "Saved {} times, cancelled {} times",
                self.saved,
                self.cancelled
            ),
        ]
        .spacing(16)
        .align_x(Center)
        .max_width(360);

        center(form).padding(20).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced::advanced::widget::operation::focusable::{find_focused, focus};
    use iced::keyboard::key::Named;
    use iced::widget::Id;
    use iced_test::{Simulator, simulator};

    /// The stops of the Tab key, in the order of the view.
    const ORDER: [&str; 8] = [
        "name", "agree", "notify", "monthly", "yearly", "size", "save",
        "cancel",
    ];

    fn focused(ui: &mut Simulator<'_, Message>) -> Option<Id> {
        ui.operate(find_focused())
    }

    #[test]
    fn tab_follows_the_order_of_the_view() {
        let form = Form::default();
        let mut ui = simulator(form.view());

        assert_eq!(focused(&mut ui), None, "nothing has the focus at first");

        for id in ORDER {
            let _ = ui.tap_key(Named::Tab);
            assert_eq!(focused(&mut ui), Some(Id::new(id)));
        }

        // After the last stop nothing has the focus; one more Tab starts over
        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), None);

        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new(ORDER[0])));
    }

    #[test]
    fn shift_tab_goes_back() {
        use iced::keyboard;
        use iced::{Event, keyboard::Modifiers};

        let form = Form::default();
        let mut ui = simulator(form.view());

        for id in ORDER.iter().rev() {
            let _ =
                ui.simulate([Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(Named::Tab),
                    modified_key: keyboard::Key::Named(Named::Tab),
                    physical_key: keyboard::key::Physical::Unidentified(
                        keyboard::key::NativeCode::Unidentified,
                    ),
                    location: keyboard::Location::Standard,
                    modifiers: Modifiers::SHIFT,
                    text: None,
                    repeat: false,
                })]);

            assert_eq!(focused(&mut ui), Some(Id::new(id)));
        }
    }

    #[test]
    fn space_and_enter_press_the_focused_control() {
        let mut form = Form::default();
        let mut ui = simulator(form.view());

        let mut press = |id: &'static str, key: Named| {
            let _: Option<()> = ui.operate(focus(Id::new(id)));
            let _ = ui.tap_key(key);
        };

        press("agree", Named::Space);
        press("notify", Named::Enter);
        press("yearly", Named::Space);
        press("save", Named::Enter);
        press("save", Named::Space);
        press("cancel", Named::Space);
        // The pick list: the arrows choose without opening it
        press("size", Named::ArrowDown);

        for message in ui.into_messages() {
            form.update(message);
        }

        assert!(form.agreed);
        assert!(form.notify);
        assert_eq!(form.plan, Plan::Yearly);
        assert_eq!((form.saved, form.cancelled), (2, 1));
        assert_eq!(form.size, Size::Large);
    }

    #[test]
    fn the_pick_list_opens_and_chooses_with_the_keyboard() {
        let mut form = Form::default();
        let mut ui = simulator(form.view());

        let _: Option<()> = ui.operate(focus(Id::new("size")));
        let _ = ui.tap_key(Named::Enter); // opens on "Medium"
        let _ = ui.tap_key(Named::ArrowUp); // "Small"
        let _ = ui.tap_key(Named::Enter); // chooses it and closes

        for message in ui.into_messages() {
            form.update(message);
        }

        assert_eq!(form.size, Size::Small);
    }

    #[test]
    fn a_key_does_nothing_without_the_focus() {
        let mut form = Form::default();
        let mut ui = simulator(form.view());

        let _ = ui.tap_key(Named::Space);
        let _ = ui.tap_key(Named::Enter);

        for message in ui.into_messages() {
            form.update(message);
        }

        assert_eq!((form.saved, form.cancelled, form.agreed), (0, 0, false));
    }

    #[test]
    fn a_click_elsewhere_takes_the_focus_away() {
        let form = Form::default();
        let mut ui = simulator(form.view());

        let _: Option<()> = ui.operate(focus(Id::new("save")));
        assert_eq!(focused(&mut ui), Some(Id::new("save")));

        let _ = ui.click("Cancel").expect("the Cancel button is there");
        assert_eq!(focused(&mut ui), None);
    }

    #[test]
    fn the_focused_button_draws_a_ring() {
        use iced::Theme;
        use iced::widget::button::{Status, primary};

        let theme = Theme::Light;
        let active = primary(&theme, Status::Active);
        let focused = primary(&theme, Status::Focused);

        assert_eq!(active.border.width, 0.0);
        assert_eq!(focused.border.width, 2.0);
        assert_eq!(
            focused.border.color,
            theme.extended_palette().background.base.text
        );
    }
}
