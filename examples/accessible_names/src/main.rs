//! Names for a screen reader, and a tree that follows scrolling.
//!
//! A list of folders as an application draws it: an arrow that is only a
//! picture, a box without a label beside the folder's name, a switch
//! beside its sentence, a field under its heading, and a button at the
//! end of a list that has to be scrolled. Each control says what it is
//! called with `.accessible_name(..)`; the tests read the tree a screen
//! reader is given and press through it.
use iced::widget::{
    button, checkbox, column, container, progress_bar, row, scrollable, space,
    text, text_input, toggler,
};
use iced::{Center, Element, Fill};

pub fn main() -> iced::Result {
    iced::application(Folders::default, Folders::update, Folders::view)
        .title("Accessible names")
        .window_size((420.0, 360.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tick {
    No,
    Yes,
    InPart,
}

#[derive(Debug)]
struct Folder {
    name: &'static str,
    tick: Tick,
    is_open: bool,
}

#[derive(Debug)]
struct Folders {
    folders: Vec<Folder>,
    email: String,
    password: String,
    starts_at_login: bool,
    reports: u32,
}

impl Default for Folders {
    fn default() -> Self {
        let folder = |name, tick| Folder {
            name,
            tick,
            is_open: false,
        };

        Self {
            folders: vec![
                folder("Work", Tick::InPart),
                folder("Photos", Tick::Yes),
                folder("Old", Tick::No),
                folder("Music", Tick::Yes),
                folder("Letters", Tick::Yes),
                folder("Taxes", Tick::No),
                folder("Recipes", Tick::Yes),
                folder("Drafts", Tick::No),
            ],
            email: String::new(),
            password: String::new(),
            starts_at_login: false,
            reports: 0,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Opened(usize),
    Ticked(usize),
    Email(String),
    Password(String),
    StartsAtLogin(bool),
    ReportSent,
}

impl Folders {
    fn update(&mut self, message: Message) {
        match message {
            Message::Opened(index) => {
                self.folders[index].is_open = !self.folders[index].is_open;
            }
            Message::Ticked(index) => {
                let folder = &mut self.folders[index];

                folder.tick = match folder.tick {
                    Tick::Yes => Tick::No,
                    Tick::No | Tick::InPart => Tick::Yes,
                };
            }
            Message::Email(email) => self.email = email,
            Message::Password(password) => self.password = password,
            Message::StartsAtLogin(on) => self.starts_at_login = on,
            Message::ReportSent => self.reports += 1,
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let folders = self.folders.iter().enumerate().map(|(index, folder)| {
            // An arrow that is a picture: without a name a screen reader
            // says "button" and nothing else
            let arrow = button(space().width(10).height(10))
                .accessible_name(format!(
                    "{} {}",
                    if folder.is_open { "Close" } else { "Open" },
                    folder.name
                ))
                .on_press(Message::Opened(index));

            // A box without a label of its own: the name of the folder
            // stands beside it, as a text
            let tick = checkbox(match folder.tick {
                Tick::No => checkbox::State::Unchecked,
                Tick::Yes => checkbox::State::Checked,
                Tick::InPart => checkbox::State::Indeterminate,
            })
            .accessible_name(format!("Sync {}", folder.name))
            .on_toggle(move |_| Message::Ticked(index));

            row![arrow, tick, text(folder.name)]
                .spacing(10)
                .align_y(Center)
                .into()
        });

        let page = column![
            text("Email"),
            text_input("you@example.com", &self.email)
                .accessible_name("Email")
                .on_input(Message::Email),
            text_input("", &self.password)
                .secure(true)
                .accessible_name("Password")
                .on_input(Message::Password),
            row![
                text("Start at login"),
                toggler(self.starts_at_login)
                    .accessible_name("Start at login")
                    .on_toggle(Message::StartsAtLogin),
            ]
            .spacing(10)
            .align_y(Center),
            progress_bar(0.0..=100.0, 34.0)
                .girth(8)
                .accessible_name("Uploading notes.txt"),
            text("What to sync"),
            column(folders).spacing(6),
            text(format!("{} reports sent", self.reports)),
            button("Send a report").on_press(Message::ReportSent),
        ]
        .spacing(14)
        .padding(20)
        .width(Fill);

        container(scrollable(page).id("page").height(Fill)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced::advanced::widget::operation::scrollable::{
        AbsoluteOffset, scroll_to,
    };
    use iced::widget::Id;
    use iced::{Settings, Size};
    use iced_test::runtime::accessibility::{Role, Tree};
    use iced_test::{Simulator, simulator};

    const WINDOW: Size = Size::new(420.0, 360.0);

    fn window(folders: &Folders) -> Simulator<'_, Message> {
        Simulator::with_size(Settings::default(), WINDOW, folders.view())
    }

    fn named(tree: &Tree, role: Role, name: &str) -> usize {
        tree.nodes
            .iter()
            .position(|node| {
                node.role == role && node.label.as_deref() == Some(name)
            })
            .unwrap_or_else(|| {
                panic!("no {role:?} named {name:?} in\n{}", tree.describe())
            })
    }

    #[test]
    fn every_control_has_a_name() {
        let folders = Folders::default();
        let mut ui = window(&folders);
        let tree = ui.accessibility("Accessible names");

        let described = tree.describe();
        let top = described.lines().take(12).collect::<Vec<_>>().join("\n");

        assert_eq!(
            top,
            r#"Window "Accessible names"
  ScrollArea scrolled=0,0
    Text "Email"
    TextInput "Email" value=""
    SecureTextInput "Password"
    Text "Start at login"
    Switch "Start at login" toggled=No
    ProgressBar "Uploading notes.txt" 34 in 0..=100
    Text "What to sync"
    Button "Open Work"
    CheckBox "Sync Work" toggled=Mixed
    Text "Work""#,
            "\n{described}"
        );

        // The field is called by its name; its example is beside it
        let email = &tree.nodes[named(&tree, Role::TextInput, "Email")];
        assert_eq!(email.placeholder.as_deref(), Some("you@example.com"));

        // No button, box, switch, field or bar is without a name
        for node in &tree.nodes {
            if !matches!(node.role, Role::Text | Role::ScrollArea) {
                assert!(
                    node.label.as_deref().is_some_and(|name| !name.is_empty()),
                    "{:?} has no name in\n{described}",
                    node.role
                );
            }
        }
    }

    #[test]
    fn a_secret_is_not_told() {
        let mut folders = Folders::default();
        folders.update(Message::Password("correct horse".to_owned()));
        folders.update(Message::Email("ada@example.com".to_owned()));

        let mut ui = window(&folders);
        let tree = ui.accessibility("Accessible names");

        let secret =
            &tree.nodes[named(&tree, Role::SecureTextInput, "Password")];
        assert_eq!(secret.value, None);
        assert!(!tree.describe().contains("correct horse"));

        let email = &tree.nodes[named(&tree, Role::TextInput, "Email")];
        assert_eq!(email.value.as_deref(), Some("ada@example.com"));
    }

    #[test]
    fn the_tree_follows_scrolling() {
        let folders = Folders::default();
        let mut ui = window(&folders);

        // At the start the button at the end of the list is below the
        // window, and the tree says so
        let tree = ui.accessibility("Accessible names");
        let report = named(&tree, Role::Button, "Send a report");
        let before = tree.nodes[report].bounds;

        assert!(before.y > WINDOW.height, "{before:?}");
        assert_eq!(tree.visible_bounds(report), None);
        assert!(
            tree.describe()
                .contains("Button \"Send a report\" out-of-view")
        );

        let email = named(&tree, Role::TextInput, "Email");
        assert!(tree.visible_bounds(email).is_some());

        // Scrolled by 150: everything inside is told 150 higher, where it
        // is drawn
        let _: Option<()> = ui.operate(scroll_to(
            Id::new("page"),
            AbsoluteOffset {
                x: None,
                y: Some(150.0),
            },
        ));

        let tree = ui.accessibility("Accessible names");
        let after = tree.nodes[report].bounds;

        assert_eq!(after.y, before.y - 150.0);
        assert_eq!(tree.nodes[report].layout, before, "the layout stays");

        let area = &tree.nodes[tree.scroll_areas(report)[0]];
        assert_eq!(area.scroll.map(|scroll| scroll.offset.y), Some(150.0));
        assert_eq!(area.bounds.size(), WINDOW);

        // The field at the top is scrolled out of view now
        assert_eq!(tree.visible_bounds(email), None);

        // What the tree says is where a click finds the button (its
        // lower part is still under the edge of the window)
        let seen = tree.visible_bounds(report).expect("the button is seen");
        assert!(seen.height < after.height);
        ui.point_at(seen.center());
        let _ = ui.simulate(simulator::click());

        assert!(matches!(
            ui.into_messages().collect::<Vec<_>>()[..],
            [Message::ReportSent]
        ));
    }

    #[test]
    fn a_screen_reader_presses_what_is_below_the_fold() {
        let mut folders = Folders::default();

        let messages = {
            let mut ui = window(&folders);
            let tree = ui.accessibility("Accessible names");
            let report = named(&tree, Role::Button, "Send a report");

            assert_eq!(tree.visible_bounds(report), None);
            assert!(
                ui.accessibility_press(&tree, report),
                "the press is done, not only answered"
            );

            // It was scrolled into view, whole, at the lower edge
            let tree = ui.accessibility("Accessible names");
            let seen = tree.visible_bounds(report).expect("in view now");
            let whole = tree.nodes[report].bounds;
            assert!((seen.height - whole.height).abs() < 0.01, "{seen:?}");
            assert!(seen.y + seen.height <= WINDOW.height + 0.01);

            // And back: the box of the first folder, by its name
            let work = named(&tree, Role::CheckBox, "Sync Work");
            assert!(ui.accessibility_press(&tree, work));

            ui.into_messages().collect::<Vec<_>>()
        };

        assert!(matches!(
            messages[..],
            [Message::ReportSent, Message::Ticked(0)]
        ));

        for message in messages {
            folders.update(message);
        }

        assert_eq!(folders.reports, 1);
        assert_eq!(folders.folders[0].tick, Tick::Yes);
    }

    #[test]
    fn a_press_on_what_changed_meanwhile_is_not_done() {
        let folders = Folders::default();
        let mut ui = window(&folders);
        let tree = ui.accessibility("Accessible names");

        // A tree of another window: its nodes are other widgets here
        let mut stale = tree.clone();
        let report = named(&stale, Role::Button, "Send a report");
        stale.nodes[report].layout.y += 1.0;

        assert!(!ui.accessibility_press(&stale, report));
        assert!(!ui.accessibility_press(&tree, tree.nodes.len()));
        assert_eq!(ui.into_messages().count(), 0);
    }

    #[test]
    fn scrolling_into_view_scrolls_the_least() {
        let folders = Folders::default();
        let mut ui = window(&folders);
        let tree = ui.accessibility("Accessible names");

        // What is in view is left where it is
        let email = named(&tree, Role::TextInput, "Email");
        assert!(!ui.accessibility_reveal(&tree, email));

        let report = named(&tree, Role::Button, "Send a report");
        assert!(ui.accessibility_reveal(&tree, report));

        let tree = ui.accessibility("Accessible names");
        let seen = tree.nodes[report].bounds;
        assert!((seen.y + seen.height - WINDOW.height).abs() < 0.01);
    }
}
