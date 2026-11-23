//! Show an icon with a menu in the tray of the desktop (the notification
//! area of Windows, the menu bar of macOS, the status area of a Linux
//! desktop).
//!
//! A program has one tray icon. It is described by [`Settings`]: given once
//! when the program starts (`iced::daemon(..).tray(settings)`), and again,
//! as a whole, whenever something about it changes ([`update`]). What the
//! person does with it comes back as [`Event`]s through the shell's
//! `tray::events()` subscription.
use crate::Action as RuntimeAction;
use crate::task::{self, Task};

/// The description of the tray icon of a program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The picture.
    pub icon: Icon,

    /// What the icon is called: the name of the program, which stays the
    /// same while the [`tooltip`](Self::tooltip) tells a status. May be
    /// empty; the icon is then called by its tooltip.
    ///
    /// A screen reader says it for the icon (the accessibility title of
    /// the menu bar item on macOS, where the tooltip is only the help
    /// text). Windows keeps the first tooltip of an icon as its name, in
    /// its settings and for a screen reader: the icon is first shown with
    /// this as its tooltip. On Linux it is the title of the item.
    pub name: String,

    /// What is shown when the pointer rests on the icon. May be empty.
    ///
    /// Some Linux desktops do not show a tooltip for tray icons; put what
    /// matters in the first, disabled item of the [`Menu`] as well.
    pub tooltip: String,

    /// The menu of the icon.
    pub menu: Menu,
}

impl Settings {
    /// Creates the [`Settings`] of a tray icon with no tooltip and an empty
    /// [`Menu`].
    pub fn new(icon: Icon) -> Self {
        Self {
            icon,
            name: String::new(),
            tooltip: String::new(),
            menu: Menu::default(),
        }
    }

    /// Sets what the icon is called; see [`name`](Self#structfield.name).
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// What the desktop is told the icon is called: its name, or its
    /// tooltip when it has none.
    pub fn title(&self) -> &str {
        if self.name.is_empty() {
            &self.tooltip
        } else {
            &self.name
        }
    }

    /// Sets the tooltip.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = tooltip.into();
        self
    }

    /// Sets the [`Menu`].
    pub fn menu(mut self, menu: impl Into<Menu>) -> Self {
        self.menu = menu.into();
        self
    }
}

/// The picture of a tray icon: pixels of red, green, blue and alpha (not
/// premultiplied), a byte each, in rows from the top.
#[derive(Clone, PartialEq, Eq)]
pub struct Icon {
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

impl Icon {
    /// Creates an [`Icon`] from its pixels.
    ///
    /// A square of 32 pixels (or 64 for dense screens) suits every desktop.
    pub fn from_rgba(
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    ) -> Result<Self, IconError> {
        let expected = width as usize * height as usize * 4;

        if width == 0 || height == 0 || rgba.len() != expected {
            return Err(IconError {
                width,
                height,
                bytes: rgba.len(),
            });
        }

        Ok(Self {
            rgba,
            width,
            height,
        })
    }

    /// The pixels: red, green, blue, alpha.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
}

impl std::fmt::Debug for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Icon({}x{})", self.width, self.height)
    }
}

/// The pixels given to [`Icon::from_rgba`] are not `width * height * 4`
/// bytes, or there are none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconError {
    width: u32,
    height: u32,
    bytes: usize,
}

impl std::fmt::Display for IconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a tray icon of {}x{} pixels needs {} bytes of RGBA, not {}",
            self.width,
            self.height,
            self.width as usize * self.height as usize * 4,
            self.bytes
        )
    }
}

impl std::error::Error for IconError {}

/// The menu of a tray icon: its items from the top.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Menu {
    /// The items.
    pub items: Vec<Item>,
}

impl Menu {
    /// Creates a [`Menu`] with the given items.
    pub fn new(items: impl IntoIterator<Item = Item>) -> Self {
        Self {
            items: items.into_iter().collect(),
        }
    }

    /// Adds an [`Item`] at the end.
    pub fn push(mut self, item: Item) -> Self {
        self.items.push(item);
        self
    }
}

impl<I> From<I> for Menu
where
    I: IntoIterator<Item = Item>,
{
    fn from(items: I) -> Self {
        Self::new(items)
    }
}

/// An item of a [`Menu`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// A line of text that can be chosen.
    Button {
        /// What [`Event::MenuItem`] carries when the item is chosen.
        id: String,
        /// The text.
        label: String,
        /// Whether it can be chosen; if not, it is shown greyed.
        enabled: bool,
        /// `Some` makes it an item with a tick: whether it is ticked.
        checked: Option<bool>,
        /// A small picture before the text (16 pixels suit every desktop).
        ///
        /// Windows and macOS draw either a tick or a picture: an item
        /// with a tick shows no picture there.
        icon: Option<Icon>,
    },
    /// A line of text that opens a menu of its own beside it.
    Submenu {
        /// The text.
        label: String,
        /// Whether it can be opened; if not, it is shown greyed.
        enabled: bool,
        /// A small picture before the text.
        icon: Option<Icon>,
        /// The items of the menu it opens.
        menu: Menu,
    },
    /// A line between two groups of items.
    Separator,
}

impl Item {
    /// An item that can be chosen.
    pub fn button(id: impl Into<String>, label: impl Into<String>) -> Self {
        Item::Button {
            id: id.into(),
            label: label.into(),
            enabled: true,
            checked: None,
            icon: None,
        }
    }

    /// An item with a tick that can be chosen. Choosing it does not change
    /// the tick: the program does, with [`update`].
    pub fn check(
        id: impl Into<String>,
        label: impl Into<String>,
        checked: bool,
    ) -> Self {
        Item::Button {
            id: id.into(),
            label: label.into(),
            enabled: true,
            checked: Some(checked),
            icon: None,
        }
    }

    /// A line of text that cannot be chosen: a status, a heading.
    pub fn label(label: impl Into<String>) -> Self {
        Item::Button {
            id: String::new(),
            label: label.into(),
            enabled: false,
            checked: None,
            icon: None,
        }
    }

    /// A line of text that opens a menu of its own. The items inside are
    /// chosen like any other: [`Event::MenuItem`] carries their `id`.
    pub fn submenu(label: impl Into<String>, menu: impl Into<Menu>) -> Self {
        Item::Submenu {
            label: label.into(),
            enabled: true,
            icon: None,
            menu: menu.into(),
        }
    }

    /// A line between two groups of items.
    pub fn separator() -> Self {
        Item::Separator
    }

    /// Sets whether the item can be chosen (or opened).
    pub fn enabled(mut self, is_enabled: bool) -> Self {
        if let Item::Button { enabled, .. } | Item::Submenu { enabled, .. } =
            &mut self
        {
            *enabled = is_enabled;
        }

        self
    }

    /// Sets a small picture before the text of the item.
    pub fn icon(mut self, picture: Icon) -> Self {
        if let Item::Button { icon, .. } | Item::Submenu { icon, .. } =
            &mut self
        {
            *icon = Some(picture);
        }

        self
    }
}

/// What a person did with the tray icon.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Event {
    /// The item of the menu with this identifier was chosen.
    MenuItem(String),
    /// The icon itself was clicked (with the main button), or activated in
    /// the way of the desktop.
    ///
    /// Not every desktop tells: where a click only opens the menu (GNOME
    /// with the AppIndicator extension, macOS), this does not come, so the
    /// menu should offer the same thing ("Open").
    Click,
    /// The desktop shows tray icons, and was given this one.
    ///
    /// Comes once after the icon is made, and again when a tray appears
    /// on a desktop that had none (a panel was started, an extension
    /// turned on). A subscription that starts later is told the latest of
    /// [`Available`](Self::Available) and
    /// [`Unavailable`](Self::Unavailable) first.
    Available,
    /// There is no tray here: nothing on this desktop shows tray icons,
    /// so the person cannot see or reach the icon. A program that lives
    /// in the tray should not hide its last window now, or should say how
    /// to get it back.
    ///
    /// Linux only: no `org.kde.StatusNotifierWatcher` is on the session
    /// bus (GNOME without the AppIndicator extension, a bare window
    /// manager), or it has no host that draws the icons, or there is no
    /// session bus at all. Windows and macOS always have a place for the
    /// icon; there it comes only if the icon could not be made.
    Unavailable,
}

/// A tray operation, for the shell.
#[derive(Debug, Clone)]
pub enum Action {
    /// Show the tray icon described by the [`Settings`], or change the one
    /// that is shown into it.
    Update(Settings),
    /// Take the tray icon away.
    Remove,
}

/// Shows the tray icon described by the given [`Settings`], or changes the
/// one that is shown into it (only what differs is sent to the desktop).
pub fn update<T>(settings: Settings) -> Task<T> {
    task::effect(RuntimeAction::Tray(Action::Update(settings)))
}

/// Takes the tray icon away.
pub fn remove<T>() -> Task<T> {
    task::effect(RuntimeAction::Tray(Action::Remove))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icon_wants_four_bytes_per_pixel() {
        assert!(Icon::from_rgba(vec![0; 16], 2, 2).is_ok());
        assert!(Icon::from_rgba(vec![0; 15], 2, 2).is_err());
        assert!(Icon::from_rgba(vec![], 0, 0).is_err());

        let error = Icon::from_rgba(vec![0; 3], 1, 1).unwrap_err();
        assert_eq!(
            error.to_string(),
            "a tray icon of 1x1 pixels needs 4 bytes of RGBA, not 3"
        );
    }

    #[test]
    fn an_icon_is_called_by_its_name_or_else_by_its_tooltip() {
        let icon = Icon::from_rgba(vec![0; 4], 1, 1).unwrap();

        let plain = Settings::new(icon.clone()).tooltip("Drive: Paused");
        assert_eq!(plain.title(), "Drive: Paused");

        let named = plain.name("Drive");
        assert_eq!(named.title(), "Drive");
        assert_eq!(named.tooltip, "Drive: Paused");

        assert_eq!(Settings::new(icon).title(), "");
    }

    #[test]
    fn a_menu_is_built_from_items() {
        let menu = Menu::new([
            Item::label("Up to date"),
            Item::separator(),
            Item::button("open", "Open"),
            Item::check("pause", "Pause", true),
            Item::button("quit", "Quit").enabled(false),
        ]);

        assert_eq!(menu.items.len(), 5);
        assert_eq!(
            menu.items[0],
            Item::Button {
                id: String::new(),
                label: "Up to date".into(),
                enabled: false,
                checked: None,
                icon: None
            }
        );
        assert_eq!(
            menu.items[3],
            Item::Button {
                id: "pause".into(),
                label: "Pause".into(),
                enabled: true,
                checked: Some(true),
                icon: None
            }
        );
        assert!(matches!(
            &menu.items[4],
            Item::Button { enabled: false, .. }
        ));
    }

    #[test]
    fn an_item_can_open_a_menu_and_carry_a_picture() {
        let dot = Icon::from_rgba(vec![255; 4], 1, 1).unwrap();

        let item = Item::submenu(
            "Recent",
            [
                Item::button("a", "a.txt").icon(dot.clone()),
                Item::button("b", "b.txt"),
            ],
        )
        .icon(dot.clone())
        .enabled(false);

        let Item::Submenu {
            label,
            enabled,
            icon,
            menu,
        } = item
        else {
            panic!("not a submenu");
        };

        assert_eq!(label, "Recent");
        assert!(!enabled);
        assert_eq!(icon, Some(dot.clone()));
        assert_eq!(menu.items.len(), 2);
        assert!(matches!(
            &menu.items[0],
            Item::Button { icon: Some(icon), .. } if *icon == dot
        ));

        // A separator takes neither
        assert_eq!(Item::separator().icon(dot).enabled(false), Item::Separator);
    }
}
