//! What a screen reader is told beyond names: a status line that is said
//! when it changes, the caret and the selection in a field, whether a list
//! is open and what is in it, how far a list is scrolled, and identifiers
//! that stay with a widget while the interface changes around it.
//!
//! The tests read the tree assistive technology is given
//! (`Simulator::accessibility`) and act through it.
use iced::widget::text::Live;
use iced::widget::{
    button, column, keyed_column, pick_list, row, scrollable, slider, text,
    text_editor, text_input,
};
use iced::{Center, Element, Fill};

pub fn main() -> iced::Result {
    iced::application(Sync::default, Sync::update, Sync::view)
        .title("Screen reader")
        .window_size((460.0, 640.0))
        .run()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Often {
    Always,
    Hourly,
    Daily,
}

impl Often {
    const ALL: [Often; 3] = [Often::Always, Often::Hourly, Often::Daily];
}

impl std::fmt::Display for Often {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Often::Always => "As files change",
            Often::Hourly => "Every hour",
            Often::Daily => "Once a day",
        })
    }
}

#[derive(Debug)]
struct Sync {
    /// How many files are still to be sent; none: up to date.
    waiting: u32,
    has_notice: bool,
    name: String,
    notes: text_editor::Content,
    often: Option<Often>,
    limit: u8,
}

impl Default for Sync {
    fn default() -> Self {
        Self {
            waiting: 0,
            has_notice: false,
            name: String::new(),
            notes: text_editor::Content::new(),
            often: None,
            limit: 10,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    SyncNow,
    FileSent,
    NoticeToggled,
    Name(String),
    Notes(text_editor::Action),
    Often(Often),
    Limit(u8),
}

impl Sync {
    fn update(&mut self, message: Message) {
        match message {
            Message::SyncNow => self.waiting = 3,
            Message::FileSent => self.waiting = self.waiting.saturating_sub(1),
            Message::NoticeToggled => self.has_notice = !self.has_notice,
            Message::Name(name) => self.name = name,
            Message::Notes(action) => self.notes.perform(action),
            Message::Often(often) => self.often = Some(often),
            Message::Limit(limit) => self.limit = limit,
        }
    }

    fn status(&self) -> String {
        match self.waiting {
            0 => "Up to date".to_owned(),
            1 => "Syncing 1 file".to_owned(),
            waiting => format!("Syncing {waiting} files"),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let activity = (1..=30)
            .map(|n| text(format!("notes-{n:02}.txt was uploaded")).into());

        // The notice comes and goes above everything else. In a plain
        // `column` the widgets below it would move up and down one place
        // and lose what they hold (the focus, the caret): a `keyed_column`
        // knows each child by its key, wherever it stands.
        let notice = self
            .has_notice
            .then(|| (1, text("The server answered slowly this time.").into()));

        let rest: [(u32, Element<'_, Message>); 10] = [
            // A status line: a screen reader says it when its words change,
            // without the person going there to read it
            (0, text(self.status()).size(20).live(Live::Polite).into()),
            (
                2,
                row![
                    button("Sync now").on_press(Message::SyncNow),
                    button("One file sent").on_press_maybe(
                        (self.waiting > 0).then_some(Message::FileSent)
                    ),
                    button(if self.has_notice {
                        "Hide the notice"
                    } else {
                        "Show a notice"
                    })
                    .id("notice")
                    .on_press(Message::NoticeToggled),
                ]
                .spacing(10)
                .into(),
            ),
            (3, text("Computer name").into()),
            // The caret and the selection are told
            (
                4,
                text_input("Mac", &self.name)
                    .id("name")
                    .accessible_name("Computer name")
                    .on_input(Message::Name)
                    .into(),
            ),
            (5, text("Notes").into()),
            (
                6,
                text_editor(&self.notes)
                    .id("notes")
                    .accessible_name("Notes")
                    .height(70)
                    .on_action(Message::Notes)
                    .into(),
            ),
            // Open or closed, and the options while it is open
            (
                7,
                pick_list(Often::ALL, self.often, Message::Often)
                    .id("often")
                    .placeholder("Choose")
                    .accessible_name("How often to sync")
                    .into(),
            ),
            (
                8,
                row![
                    text!("At most {} MB a second", self.limit).width(190),
                    slider(1..=50, self.limit, Message::Limit)
                        .id("limit")
                        .accessible_name("Speed limit"),
                ]
                .spacing(10)
                .align_y(Center)
                .into(),
            ),
            (9, text("Recent activity").into()),
            // How far it is scrolled is told, and it can be scrolled
            (
                10,
                scrollable(column(activity).spacing(4).width(Fill))
                    .id("activity")
                    .height(120)
                    .into(),
            ),
        ];

        let mut rest = rest.into_iter();
        let status = rest.next();

        keyed_column(status.into_iter().chain(notice).chain(rest))
            .spacing(12)
            .padding(20)
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced::advanced::widget::operation::focusable::focus;
    use iced::keyboard::key::Named;
    use iced::widget::Id;
    use iced::{Settings, Size, Vector};
    use iced_test::runtime::accessibility::{Node, Role, Tree};
    use iced_test::{Simulator, simulator};

    fn named<'a>(tree: &'a Tree, role: Role, name: &str) -> &'a Node {
        &tree.nodes[index(tree, role, name)]
    }

    fn index(tree: &Tree, role: Role, name: &str) -> usize {
        tree.nodes
            .iter()
            .position(|node| {
                node.role == role && node.label.as_deref() == Some(name)
            })
            .unwrap_or_else(|| {
                panic!("no {role:?} named {name:?} in\n{}", tree.describe())
            })
    }

    /// Hands the messages of the simulator to the state.
    macro_rules! apply {
        ($sync:ident, $ui:ident) => {
            for message in $ui.into_messages() {
                $sync.update(message);
            }
        };
    }

    #[test]
    fn the_tree_tells_more_than_names() {
        let mut sync = Sync::default();
        sync.update(Message::Name("Ada".to_owned()));

        let mut ui = simulator(sync.view());
        let tree = ui.accessibility("Screen reader");

        let top: Vec<_> = tree
            .describe()
            .lines()
            .take(15)
            .map(str::to_owned)
            .collect();

        assert_eq!(
            top.join("\n"),
            r#"Window "Screen reader"
  Text "Up to date" live=Polite
  Button "Sync now"
  Button "One file sent" disabled
  Button "Show a notice"
  Text "Computer name"
  TextInput "Computer name" value="Ada" caret=0
  Text "Notes"
  TextEditor "Notes" value="" caret=0
  ComboBox "How often to sync" closed
  Text "At most 10 MB a second"
  Slider "Speed limit" 10 in 1..=50
  Text "Recent activity"
  ScrollArea scrolled=0,0
    Text "notes-01.txt was uploaded""#
        );
    }

    #[test]
    fn a_status_line_is_live_and_stays_the_same_node_while_its_words_change() {
        let mut sync = Sync::default();

        let before = {
            let mut ui = simulator(sync.view());
            let tree = ui.accessibility("Screen reader");
            let status = named(&tree, Role::Text, "Up to date").clone();

            let _ = ui.click("Sync now").expect("the button is there");
            apply!(sync, ui);

            status
        };

        assert_eq!(before.live, Some(Live::Polite));

        let mut ui = simulator(sync.view());
        let tree = ui.accessibility("Screen reader");
        let after = named(&tree, Role::Text, "Syncing 3 files");

        assert_eq!(after.live, Some(Live::Polite));
        assert_eq!(
            after.id, before.id,
            "a reader compares the two to know what to say"
        );

        // No other text is one
        assert_eq!(tree.nodes.iter().filter(|n| n.live.is_some()).count(), 1);
    }

    #[test]
    fn identifiers_survive_a_new_layout() {
        let mut sync = Sync::default();
        sync.update(Message::Name("Ada".to_owned()));

        let ids = |tree: &Tree| {
            [
                named(tree, Role::Text, "Computer name").id,
                named(tree, Role::TextInput, "Computer name").id,
                named(tree, Role::TextEditor, "Notes").id,
                named(tree, Role::ComboBox, "How often to sync").id,
                named(tree, Role::Slider, "Speed limit").id,
                named(tree, Role::Button, "Sync now").id,
                named(tree, Role::Text, "notes-07.txt was uploaded").id,
                tree.with_role(Role::ScrollArea).next().unwrap().id,
                tree.nodes[Tree::ROOT].id,
            ]
        };

        let before = {
            let mut ui = simulator(sync.view());
            ids(&ui.accessibility("Screen reader"))
        };

        // Every identifier is its own
        let mut distinct = before.to_vec();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), before.len());

        // A notice appears above everything, the window becomes narrow (the
        // sentences wrap, everything moves), the name is typed further,
        // the status changes and the list is scrolled
        sync.update(Message::NoticeToggled);
        sync.update(Message::Name("Ada Lovelace".to_owned()));
        sync.update(Message::SyncNow);
        sync.update(Message::Limit(30));

        let mut ui = Simulator::with_size(
            Settings::default(),
            Size::new(300.0, 900.0),
            sync.view(),
        );

        let tree = ui.accessibility("Screen reader");
        let area = tree
            .nodes
            .iter()
            .position(|node| node.role == Role::ScrollArea)
            .unwrap();
        assert!(ui.accessibility_scroll(&tree, area, Vector::new(0.0, 90.0)));

        let tree = ui.accessibility("Screen reader");
        assert!(tree.describe().contains("The server answered slowly"));
        assert!(tree.describe().contains("scrolled=0,90"));

        assert_eq!(ids(&tree), before);

        // The button that was given an `Id` is the same button although it
        // is called something else now
        let again =
            simulator(Sync::default().view()).accessibility("Screen reader");
        assert_eq!(
            named(&tree, Role::Button, "Hide the notice").id,
            named(&again, Role::Button, "Show a notice").id,
        );
    }

    #[test]
    fn two_of_a_kind_with_one_name_are_told_apart_by_their_order() {
        use iced::widget::{button, column};

        let rows = |names: &[&str]| -> Element<'static, ()> {
            column(names.iter().map(|name| {
                row![text(name.to_string()), button("Remove").on_press(())]
                    .into()
            }))
            .into()
        };

        let removes = |view| {
            let tree = simulator(view).accessibility("List");

            tree.with_role(Role::Button)
                .map(|b| b.id)
                .collect::<Vec<_>>()
        };

        let three = removes(rows(&["a.txt", "b.txt", "c.txt"]));
        assert!(three[0] != three[1] && three[1] != three[2]);

        // Without the last row the first two are who they were
        let two = removes(rows(&["a.txt", "b.txt"]));
        assert_eq!(two[..], three[..2]);
    }

    #[test]
    fn the_caret_and_the_selection_of_a_field_are_told() {
        let mut sync = Sync::default();

        {
            let mut ui = simulator(sync.view());
            let _: Option<()> = ui.operate(focus(Id::new("name")));
            let _ = ui.typewrite("Zoë");
            apply!(sync, ui);
        }

        assert_eq!(sync.name, "Zoë");

        let mut ui = simulator(sync.view());
        let _: Option<()> = ui.operate(focus(Id::new("name")));
        let _ = ui.tap_key(Named::End);

        let tree = ui.accessibility("Screen reader");
        let field = named(&tree, Role::TextInput, "Computer name");
        assert!(field.is_focused);
        assert_eq!(
            field.selection.map(|s| (s.anchor, s.focus)),
            Some((4, 4)),
            "in bytes: the e with two dots is two"
        );

        let _ = ui.tap_key(Named::ArrowLeft);
        let tree = ui.accessibility("Screen reader");
        let field = named(&tree, Role::TextInput, "Computer name");
        assert_eq!(field.selection.map(|s| s.focus), Some(2));

        // Everything selected
        let _: Option<()> = ui.operate(
            iced::advanced::widget::operation::text_input::select_all(Id::new(
                "name",
            )),
        );
        let tree = ui.accessibility("Screen reader");
        let field = named(&tree, Role::TextInput, "Computer name");
        assert_eq!(field.selection.map(|s| s.range()), Some(0..4));
        assert!(tree.describe().contains("selected=0..4"));
    }

    #[test]
    fn the_caret_of_an_editor_is_told_over_its_lines() {
        let mut sync = Sync::default();

        {
            let mut ui = simulator(sync.view());
            let _: Option<()> = ui.operate(focus(Id::new("notes")));
            let _ = ui.typewrite("one");
            let _ = ui.tap_key(Named::Enter);
            let _ = ui.typewrite("two");
            apply!(sync, ui);
        }

        let mut ui = simulator(sync.view());
        let tree = ui.accessibility("Screen reader");
        let notes = named(&tree, Role::TextEditor, "Notes");

        assert_eq!(notes.value.as_deref(), Some("one\ntwo"));
        assert_eq!(notes.selection.map(|s| (s.anchor, s.focus)), Some((7, 7)));
        assert!(notes.is_focusable);
    }

    #[test]
    fn a_pick_list_says_that_it_is_open_and_what_is_in_it() {
        let mut sync = Sync::default();
        let mut ui = simulator(sync.view());

        let closed = ui.accessibility("Screen reader");
        let list = named(&closed, Role::ComboBox, "How often to sync");
        assert_eq!(list.expanded, Some(false));
        assert_eq!(closed.with_role(Role::Menu).count(), 0);

        let _: Option<()> = ui.operate(focus(Id::new("often")));
        let _ = ui.tap_key(Named::Enter);
        let _ = ui.tap_key(Named::ArrowDown);

        let open = ui.accessibility("Screen reader");
        let list = named(&open, Role::ComboBox, "How often to sync");
        assert_eq!(list.expanded, Some(true));
        assert_eq!(
            list.id,
            named(&closed, Role::ComboBox, "How often to sync").id
        );

        let described = open.describe();
        let menu = described
            .lines()
            .skip_while(|line| !line.trim_start().starts_with("Menu"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            menu,
            r#"  Menu
    ScrollArea scrolled=0,0
      MenuItem "As files change" chosen
      MenuItem "Every hour"
      MenuItem "Once a day""#
        );

        // An option is pressed through the tree, as a screen reader does
        let daily = index(&open, Role::MenuItem, "Once a day");
        assert!(ui.accessibility_press(&open, daily));

        let after = ui.accessibility("Screen reader");
        assert_eq!(
            named(&after, Role::ComboBox, "How often to sync").expanded,
            Some(false)
        );

        apply!(sync, ui);
        assert_eq!(sync.often, Some(Often::Daily));
    }

    #[test]
    fn a_slider_is_moved_and_a_list_is_scrolled_through_the_tree() {
        let mut sync = Sync::default();
        let mut ui = simulator(sync.view());
        let tree = ui.accessibility("Screen reader");

        let limit = index(&tree, Role::Slider, "Speed limit");
        assert_eq!(tree.nodes[limit].range, Some((10.0, 1.0, 50.0)));
        assert_eq!(tree.nodes[limit].step, Some(1.0));
        assert!(ui.accessibility_set_number(&tree, limit, 25.0));

        // Not a slider: nothing happens
        let name = index(&tree, Role::TextInput, "Computer name");
        assert!(!ui.accessibility_set_number(&tree, name, 25.0));

        let area = tree
            .nodes
            .iter()
            .position(|node| node.role == Role::ScrollArea)
            .unwrap();
        assert!(ui.accessibility_scroll(&tree, area, Vector::new(0.0, 60.0)));
        assert!(!ui.accessibility_scroll(&tree, limit, Vector::new(0.0, 60.0)));

        let scrolled = ui.accessibility("Screen reader");
        assert_eq!(
            scrolled.nodes[area].scroll.map(|scroll| scroll.offset.y),
            Some(60.0)
        );
        assert_eq!(scrolled.nodes[area].id, tree.nodes[area].id);

        apply!(sync, ui);
        assert_eq!(sync.limit, 25);
    }
}
