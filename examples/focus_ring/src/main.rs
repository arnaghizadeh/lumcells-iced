use iced::widget::{
    button, center, checkbox, column, combo_box, container, focus_trap, opaque,
    pick_list, radio, row, scrollable, slider, stack, text, text_editor,
    text_input, toggler,
};
use iced::{Center, Color, Element, Fill};

pub fn main() -> iced::Result {
    iced::application(Form::default, Form::update, Form::view)
        .title("Focus ring")
        .window_size((460.0, 820.0))
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

const LANGUAGES: [&str; 4] = ["Dutch", "English", "Persian", "Spanish"];

const TERMS: &str = "These are the terms. They are longer than the box \
    they are shown in, and nothing inside them takes the keyboard focus, so \
    the box itself is a stop of the Tab key: with the focus on it the \
    arrows, Page Up, Page Down, Space, Home and End scroll it. A person \
    without a pointer can read them to their end before agreeing with them, \
    which is the least that can be asked of terms.";

#[derive(Debug)]
struct Form {
    name: String,
    agreed: bool,
    notify: bool,
    plan: Plan,
    size: Size,
    volume: u8,
    languages: combo_box::State<&'static str>,
    language: Option<&'static str>,
    notes: text_editor::Content,
    saved: u32,
    cancelled: u32,
    /// The dialog that asks before everything is cleared: a modal layer.
    is_asking: bool,
    cleared: u32,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            name: String::new(),
            agreed: false,
            notify: false,
            plan: Plan::default(),
            size: Size::default(),
            volume: 40,
            languages: combo_box::State::new(LANGUAGES.to_vec()),
            language: None,
            notes: text_editor::Content::new(),
            saved: 0,
            cancelled: 0,
            is_asking: false,
            cleared: 0,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    NameChanged(String),
    Agreed(bool),
    Notify(bool),
    PlanChosen(Plan),
    SizeChosen(Size),
    VolumeChanged(u8),
    LanguageChosen(&'static str),
    Notes(text_editor::Action),
    Save,
    Cancel,
    Clear,
    ClearConfirmed,
    ClearRefused,
}

impl Form {
    fn update(&mut self, message: Message) {
        match message {
            Message::NameChanged(name) => self.name = name,
            Message::Agreed(agreed) => self.agreed = agreed,
            Message::Notify(notify) => self.notify = notify,
            Message::PlanChosen(plan) => self.plan = plan,
            Message::SizeChosen(size) => self.size = size,
            Message::VolumeChanged(volume) => self.volume = volume,
            Message::LanguageChosen(language) => {
                self.language = Some(language);
            }
            Message::Notes(action) => self.notes.perform(action),
            Message::Save => self.saved += 1,
            Message::Cancel => self.cancelled += 1,
            Message::Clear => self.is_asking = true,
            Message::ClearRefused => self.is_asking = false,
            Message::ClearConfirmed => {
                *self = Self {
                    cleared: self.cleared + 1,
                    ..Self::default()
                };
            }
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
            // With the focus: the arrows, Page Up, Page Down, Home, End
            row![
                text!("Volume {}", self.volume).width(90),
                slider(0..=100, self.volume, Message::VolumeChanged)
                    .id("volume")
                    .accessible_name("Volume"),
            ]
            .spacing(10)
            .align_y(Center),
            // Nothing inside takes the focus, so the scrollable itself does
            scrollable(text(TERMS)).id("terms").height(70),
            // A field with a list: open while it has the focus
            combo_box(
                &self.languages,
                "Language",
                self.language.as_ref(),
                Message::LanguageChosen
            )
            .id("language"),
            text_editor(&self.notes)
                .id("notes")
                .placeholder("Notes")
                .height(70)
                .on_action(Message::Notes),
            // A field that cannot be typed in is not a stop of the Tab key
            text_input("Not yours to change", "").id("read-only"),
            row![
                button("Save").id("save").on_press(Message::Save),
                button("Cancel")
                    .id("cancel")
                    .style(button::secondary)
                    .on_press(Message::Cancel),
                // A disabled button is not a stop of the Tab key
                button("Not yet").id("disabled"),
                button("Clear…")
                    .id("clear")
                    .style(button::danger)
                    .on_press(Message::Clear),
            ]
            .spacing(10),
            text!(
                "Saved {} times, cancelled {} times, cleared {} times",
                self.saved,
                self.cancelled,
                self.cleared
            ),
        ]
        .spacing(16)
        .align_x(Center)
        .max_width(360);

        let form = center(form).padding(20);

        if !self.is_asking {
            return form.into();
        }

        // A modal layer. `opaque` keeps the pointer from what is below it,
        // `focus_trap` the keyboard: Tab goes round inside the dialog, the
        // keys do not reach the form, and Escape is an answer.
        let dialog = container(
            column![
                text("Clear everything you filled in?").size(18),
                row![
                    button("Keep it")
                        .id("keep")
                        .style(button::secondary)
                        .on_press(Message::ClearRefused),
                    button("Clear it")
                        .id("confirm")
                        .style(button::danger)
                        .on_press(Message::ClearConfirmed),
                ]
                .spacing(10),
            ]
            .spacing(16)
            .align_x(Center),
        )
        .padding(24)
        .style(container::rounded_box);

        stack![
            form,
            opaque(
                center(focus_trap(dialog).on_escape(Message::ClearRefused))
                    .width(Fill)
                    .height(Fill)
                    .style(|_| container::background(Color {
                        a: 0.6,
                        ..Color::BLACK
                    }))
            )
        ]
        .into()
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
    const ORDER: [&str; 13] = [
        "name", "agree", "notify", "monthly", "yearly", "size", "volume",
        "terms", "language", "notes", "save", "cancel", "clear",
    ];

    fn focused<M>(ui: &mut Simulator<'_, M>) -> Option<Id> {
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
        let form = Form::default();
        let mut ui = simulator(form.view());

        for id in ORDER.iter().rev() {
            let _ = ui.simulate(shift_tab());

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
    fn the_form_is_told_to_assistive_technology() {
        let mut form = Form::default();
        form.update(Message::NameChanged("Ada".to_owned()));
        form.update(Message::Agreed(true));

        let mut ui = simulator(form.view());
        let _: Option<()> = ui.operate(focus(Id::new("save")));

        let tree = ui.accessibility("Focus ring");

        assert_eq!(
            tree.describe(),
            r#"Window "Focus ring"
  Text "Press Tab to move, Space or Enter to press"
  TextInput "Your name" value="Ada"
  CheckBox "I agree with the terms" toggled=Yes
  Switch "Tell me when it is done" toggled=No
  RadioButton "Monthly" toggled=Yes
  RadioButton "Yearly" toggled=No
  ComboBox value="Medium"
  Text "Volume 40"
  Slider "Volume" 40 in 0..=100
  ScrollArea scrolled=0,0
    Text "These are the terms. They are longer than the box they are shown in, and nothing inside them takes the keyboard focus, so the box itself is a stop of the Tab key: with the focus on it the arrows, Page Up, Page Down, Space, Home and End scroll it. A person without a pointer can read them to their end before agreeing with them, which is the least that can be asked of terms."
  TextInput "Language" value=""
  TextInput "Not yours to change" value="" disabled
  Button "Save" focused
  Button "Cancel"
  Button "Not yet" disabled
  Button "Clear…"
  Text "Saved 0 times, cancelled 0 times, cleared 0 times"
"#
        );

        // What can take the focus says so, with where it is
        use iced_test::runtime::accessibility::Role;

        let save = &tree.nodes[tree.focused().expect("a node is focused")];
        assert_eq!(save.role, Role::Button);
        assert!(save.bounds.width > 0.0 && save.bounds.height > 0.0);

        assert!(tree.with_role(Role::Button).take(2).all(|b| b.is_focusable));
        assert!(!tree.with_role(Role::Button).nth(2).unwrap().is_focusable);
        assert!(
            tree.with_role(Role::TextInput)
                .all(|field| field.is_focusable)
        );
    }

    fn shift_tab() -> [iced::Event; 1] {
        use iced::keyboard;

        [iced::Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(Named::Tab),
            modified_key: keyboard::Key::Named(Named::Tab),
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::SHIFT,
            text: None,
            repeat: false,
        })]
    }

    #[test]
    fn the_keys_move_a_focused_slider() {
        let mut form = Form::default();
        let mut volumes = Vec::new();

        for key in [
            Named::ArrowRight,
            Named::ArrowUp,
            Named::ArrowLeft,
            Named::PageUp,
            Named::PageDown,
            Named::End,
            Named::ArrowRight, // already at the end: nothing
            Named::Home,
        ] {
            let mut ui = simulator(form.view());
            let _: Option<()> = ui.operate(focus(Id::new("volume")));
            let _ = ui.tap_key(key);

            for message in ui.into_messages() {
                form.update(message);
            }

            volumes.push(form.volume);
        }

        assert_eq!(volumes, [41, 42, 41, 51, 41, 100, 100, 0]);
    }

    #[test]
    fn a_slider_without_the_focus_ignores_the_keys() {
        let mut form = Form::default();
        let mut ui = simulator(form.view());

        let _ = ui.tap_key(Named::ArrowRight);
        let _ = ui.tap_key(Named::End);

        for message in ui.into_messages() {
            form.update(message);
        }

        assert_eq!(form.volume, 40);
    }

    #[test]
    fn the_keys_scroll_a_focused_scrollable() {
        use iced_test::runtime::accessibility::Role;

        let form = Form::default();
        let mut ui = simulator(form.view());

        let scrolled = |ui: &mut Simulator<'_, Message>| {
            let tree = ui.accessibility("Focus ring");
            let area = tree.with_role(Role::ScrollArea).next().unwrap();
            let scroll = area.scroll.unwrap();

            (scroll.offset.y, scroll.content.height - area.bounds.height)
        };

        // Without the focus the keys are not for it
        let _ = ui.tap_key(Named::ArrowDown);
        assert_eq!(scrolled(&mut ui).0, 0.0);

        let _: Option<()> = ui.operate(focus(Id::new("terms")));

        let _ = ui.tap_key(Named::ArrowDown);
        assert_eq!(scrolled(&mut ui).0, 40.0);

        let _ = ui.tap_key(Named::ArrowUp);
        assert_eq!(scrolled(&mut ui).0, 0.0);

        let _ = ui.tap_key(Named::PageDown);
        assert_eq!(scrolled(&mut ui).0, 40.0, "a page is the box less a line");

        let _ = ui.tap_key(Named::End);
        let (offset, room) = scrolled(&mut ui);
        assert!(room > 40.0, "the terms are longer than their box");
        assert!((offset - room).abs() < 1.0, "{offset} of {room}");

        let _ = ui.tap_key(Named::Home);
        assert_eq!(scrolled(&mut ui).0, 0.0);

        // Tab still leaves it
        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("language")));
    }

    #[test]
    fn a_scrollable_with_a_focusable_widget_inside_is_not_a_stop() {
        use iced::widget::{button, column, scrollable, space};

        let view: Element<'_, ()> = scrollable(column![
            button("First").id("first").on_press(()),
            space().height(2000),
            button("Last").id("last").on_press(()),
        ])
        .id("list")
        .height(200)
        .into();

        let mut ui = simulator(view);

        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("first")));

        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("last")));

        // The focus under the fold was brought into view
        use iced_test::runtime::accessibility::Role;

        let tree = ui.accessibility("List");
        let last = tree.focused().expect("the last button has the focus");
        assert_eq!(tree.nodes[last].role, Role::Button);
        assert!(
            tree.visible_bounds(last).is_some_and(|seen| {
                seen.height > tree.nodes[last].bounds.height - 1.0
            }),
            "{}",
            tree.describe()
        );
    }

    #[test]
    fn a_scrollable_with_nothing_to_scroll_is_not_a_stop() {
        use iced::widget::{button, column, scrollable};

        let view: Element<'_, ()> = column![
            scrollable("Short").id("short").height(200),
            button("After").id("after").on_press(()),
        ]
        .into();

        let mut ui = simulator(view);

        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("after")));
    }

    #[test]
    fn the_combo_box_takes_the_focus_chooses_and_lets_tab_leave() {
        let mut form = Form::default();

        {
            let mut ui = simulator(form.view());

            let _: Option<()> = ui.operate(focus(Id::new("language")));
            let _ = ui.typewrite("s"); // "English", "Persian", "Spanish"
            let _ = ui.tap_key(Named::ArrowDown);
            let _ = ui.tap_key(Named::ArrowDown);
            let _ = ui.tap_key(Named::Enter);

            for message in ui.into_messages() {
                form.update(message);
            }
        }

        assert_eq!(form.language, Some("Spanish"));

        // Tab is not kept by the list: it goes on to the next stop
        let mut ui = simulator(form.view());

        let _: Option<()> = ui.operate(focus(Id::new("language")));
        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("notes")));

        let _ = ui.simulate(shift_tab());
        assert_eq!(focused(&mut ui), Some(Id::new("language")));
    }

    #[test]
    fn the_editor_takes_the_focus_is_typed_in_and_lets_tab_leave() {
        let mut form = Form::default();
        let mut ui = simulator(form.view());

        // Not typed in without the focus
        let _ = ui.typewrite("no");

        let _: Option<()> = ui.operate(focus(Id::new("notes")));
        let _ = ui.typewrite("yes");
        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("save")));

        for message in ui.into_messages() {
            form.update(message);
        }

        assert_eq!(form.notes.text().trim_end(), "yes");
    }

    #[test]
    fn the_dialog_keeps_the_focus_and_the_keys() {
        let mut form = Form::default();

        // The dialog is opened with the keyboard: "Clear…" has the focus
        {
            let mut ui = simulator(form.view());
            let _: Option<()> = ui.operate(focus(Id::new("clear")));
            let _ = ui.tap_key(Named::Enter);

            for message in ui.into_messages() {
                form.update(message);
            }
        }

        assert!(form.is_asking);

        let mut ui = simulator(form.view());

        // The widget tree of the form was built anew under the dialog, so
        // give the button below its focus back, as it has in a window
        let _: Option<()> = ui.operate(focus(Id::new("clear")));

        // Enter does not reach the button below the dialog, nor do other
        // keys reach the form
        let _ = ui.tap_key(Named::Enter);
        let _ = ui.tap_key(Named::Space);

        // Tab goes round inside the dialog, both ways, and takes the focus
        // from what is below
        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("keep")));

        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("confirm")));

        let _ = ui.tap_key(Named::Tab);
        assert_eq!(focused(&mut ui), Some(Id::new("keep")));

        let _ = ui.simulate(shift_tab());
        assert_eq!(focused(&mut ui), Some(Id::new("confirm")));

        let _ = ui.simulate(shift_tab());
        assert_eq!(focused(&mut ui), Some(Id::new("keep")));

        // Enter presses the button of the dialog that has the focus
        let _ = ui.tap_key(Named::Enter);

        let messages: Vec<_> = ui.into_messages().collect();
        assert!(
            matches!(messages[..], [Message::ClearRefused]),
            "only the dialog heard the keys: {messages:?}"
        );
    }

    #[test]
    fn escape_answers_the_dialog() {
        let mut form = Form::default();
        form.update(Message::Clear);

        let mut ui = simulator(form.view());
        let _ = ui.tap_key(Named::Escape);

        let messages: Vec<_> = ui.into_messages().collect();
        assert!(matches!(messages[..], [Message::ClearRefused]));

        // Without the dialog Escape is nobody's
        let form = Form::default();
        let mut ui = simulator(form.view());
        let _ = ui.tap_key(Named::Escape);
        assert_eq!(ui.into_messages().count(), 0);
    }

    #[test]
    fn the_focused_slider_and_scrollable_draw_a_ring() {
        use iced::Theme;
        use iced::widget::{scrollable, slider};

        let theme = Theme::Light;
        let ring = theme.extended_palette().background.base.text;

        let active = slider::default(&theme, slider::Status::Active);
        let focused = slider::default(&theme, slider::Status::Focused);
        assert_eq!(active.handle.border_width, 0.0);
        assert_eq!(focused.handle.border_width, 2.0);
        assert_eq!(focused.handle.border_color, ring);

        let focused = scrollable::default(
            &theme,
            scrollable::Status::Focused {
                is_horizontal_scrollbar_disabled: true,
                is_vertical_scrollbar_disabled: false,
            },
        );
        assert_eq!(focused.container.border.width, 2.0);
        assert_eq!(focused.container.border.color, ring);
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
