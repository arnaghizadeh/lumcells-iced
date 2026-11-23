use iced::widget::{center, checkbox, column, text};
use iced::{Element, Fill};

pub fn main() -> iced::Result {
    iced::application(Folder::default, Folder::update, Folder::view)
        .title("Three-state check box")
        .window_size((380.0, 300.0))
        .run()
}

struct Folder {
    files: Vec<(&'static str, bool)>,
}

impl Default for Folder {
    fn default() -> Self {
        Self {
            files: vec![
                ("Report.pdf", true),
                ("Budget.ods", false),
                ("Notes.txt", false),
            ],
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    All(bool),
    File(usize, bool),
}

impl Folder {
    fn update(&mut self, message: Message) {
        match message {
            Message::All(ticked) => {
                for (_, file) in &mut self.files {
                    *file = ticked;
                }
            }
            Message::File(index, ticked) => {
                self.files[index].1 = ticked;
            }
        }
    }

    /// Checked when every file is ticked, unchecked when none is,
    /// indeterminate in between.
    fn state(&self) -> checkbox::State {
        let ticked = self.files.iter().filter(|(_, ticked)| *ticked).count();

        checkbox::State::of_parts(ticked, self.files.len())
    }

    fn view(&self) -> Element<'_, Message> {
        let files = column(self.files.iter().enumerate().map(
            |(index, (name, ticked))| {
                checkbox(*ticked)
                    .label(*name)
                    .on_toggle(move |ticked| Message::File(index, ticked))
                    .into()
            },
        ))
        .spacing(10)
        .padding([0, 24]);

        let ticked = self.files.iter().filter(|(_, ticked)| *ticked).count();

        center(
            column![
                checkbox(self.state())
                    .label("Documents")
                    .on_toggle(Message::All),
                files,
                text!("{ticked} of {} will be synced", self.files.len()),
                // A box that cannot be changed shows the three states too
                checkbox(self.state()).label("The same, disabled"),
            ]
            .spacing(14)
            .width(Fill)
            .max_width(300),
        )
        .padding(20)
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use iced::Theme;
    use iced::widget::checkbox::{State, Status, primary};
    use iced_test::simulator;

    fn click(folder: &mut Folder, label: &str) {
        let mut ui = simulator(folder.view());
        let _ = ui.click(label).expect("the label is on the screen");

        for message in ui.into_messages() {
            folder.update(message);
        }
    }

    #[test]
    fn the_folder_follows_its_files() {
        let mut folder = Folder::default();
        assert_eq!(folder.state(), State::Indeterminate);

        click(&mut folder, "Budget.ods");
        assert_eq!(folder.state(), State::Indeterminate);

        click(&mut folder, "Notes.txt");
        assert_eq!(folder.state(), State::Checked);

        click(&mut folder, "Report.pdf");
        click(&mut folder, "Budget.ods");
        click(&mut folder, "Notes.txt");
        assert_eq!(folder.state(), State::Unchecked);
    }

    #[test]
    fn a_click_on_the_dash_ticks_everything() {
        let mut folder = Folder::default();
        assert_eq!(folder.state(), State::Indeterminate);

        click(&mut folder, "Documents");
        assert_eq!(folder.state(), State::Checked);
        assert!(folder.files.iter().all(|(_, ticked)| *ticked));

        click(&mut folder, "Documents");
        assert_eq!(folder.state(), State::Unchecked);
    }

    #[test]
    fn a_bool_still_makes_a_check_box() {
        assert_eq!(State::from(true), State::Checked);
        assert_eq!(State::from(false), State::Unchecked);
        assert!(State::Checked.is_checked());
        assert!(!State::Indeterminate.is_checked());
        assert_eq!(State::of_parts(0, 3), State::Unchecked);
        assert_eq!(State::of_parts(2, 3), State::Indeterminate);
        assert_eq!(State::of_parts(3, 3), State::Checked);
    }

    #[test]
    fn the_status_carries_the_state_and_the_dash_is_filled_like_a_tick() {
        let theme = Theme::Light;

        let status = Status::Active {
            state: State::Indeterminate,
        };
        assert_eq!(status.state(), State::Indeterminate);
        assert!(!status.is_checked());

        let dash = primary(&theme, status);
        let tick = primary(
            &theme,
            Status::Active {
                state: State::Checked,
            },
        );
        let empty = primary(
            &theme,
            Status::Active {
                state: State::Unchecked,
            },
        );

        assert_eq!(dash.background, tick.background);
        assert_ne!(dash.background, empty.background);
    }

    #[test]
    fn the_dash_is_drawn() -> Result<(), iced_test::Error> {
        // The box of the folder (indeterminate) and the box of a file that
        // is not ticked differ in what is drawn inside: the dash.
        let folder = Folder::default();
        let mut ui = simulator(folder.view());
        let with_dash = ui.snapshot(&Theme::Light)?;

        let mut none = Folder::default();
        none.update(Message::All(false));
        let mut ui = simulator(none.view());
        let without = ui.snapshot(&Theme::Light)?;

        let all = {
            let mut all = Folder::default();
            all.update(Message::All(true));
            all
        };
        let mut ui = simulator(all.view());
        let with_ticks = ui.snapshot(&Theme::Light)?;

        let (dash, _) = with_dash.into_rgba();
        let (empty, _) = without.into_rgba();
        let (ticks, _) = with_ticks.into_rgba();

        assert_ne!(dash, empty);
        assert_ne!(dash, ticks);

        Ok(())
    }
}
