//! Describe widgets to assistive technology.
//!
//! A widget tells what it is in its `operate`, next to what it already
//! tells there (its text, its focus): one call of
//! [`Operation::accessible`](super::Operation::accessible) with an
//! [`Accessible`]. A shell collects these calls into the tree a screen
//! reader walks. The children of the widget (what it visits in the
//! `traverse` that follows) become the children of its node.

/// What a widget is, to a person who cannot see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Role {
    /// Something that is pressed.
    Button,
    /// A box that is ticked or not (or in part).
    CheckBox,
    /// A switch that is on or off.
    Switch,
    /// One choice among a few, of which one is chosen.
    RadioButton,
    /// A list that opens to choose one of its options.
    ComboBox,
    /// A field text is typed in, with a list of options that follows what
    /// is typed.
    EditableComboBox,
    /// A bar that shows how far something is.
    ProgressBar,
    /// A field text is typed in.
    TextInput,
    /// A field a secret is typed in (a password): what is typed is not
    /// told.
    SecureTextInput,
    /// A handle on a rail that chooses a number between two ends.
    Slider,
    /// A field text is typed in over several lines.
    TextEditor,
    /// A piece of text that has more to tell than its words: a selection
    /// in it, or that it is a status worth saying when it changes
    /// ([`Accessible::live`]). Plain text needs no description.
    Text,
    /// The open list of a [`Role::ComboBox`], or a menu.
    Menu,
    /// One option of a [`Role::Menu`].
    MenuItem,
}

/// How a change of a piece of text is told to a person who does not look at
/// it: a status line ("Up to date", "Waiting for approval").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Live {
    /// Said when the screen reader has nothing else to say.
    Polite,
    /// Said at once, over what is being said.
    Assertive,
}

/// The caret and the selection in the [`value`](Accessible::value) of a
/// widget, in bytes of that text (UTF-8) from its start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Selection {
    /// Where the selection began. Equal to [`focus`](Self::focus) when
    /// nothing is selected.
    pub anchor: usize,
    /// Where the caret is: the end of the selection that moves.
    pub focus: usize,
}

impl Selection {
    /// A caret with nothing selected.
    pub fn caret(at: usize) -> Self {
        Self {
            anchor: at,
            focus: at,
        }
    }

    /// The selection as a range from its start to its end, whichever way
    /// it was made.
    pub fn range(self) -> std::ops::Range<usize> {
        self.anchor.min(self.focus)..self.anchor.max(self.focus)
    }
}

/// Whether something that can be ticked is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Toggled {
    /// It is not.
    No,
    /// It is.
    Yes,
    /// It is in part.
    Mixed,
}

impl From<bool> for Toggled {
    fn from(toggled: bool) -> Self {
        if toggled { Toggled::Yes } else { Toggled::No }
    }
}

/// The description of a widget for assistive technology.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct Accessible<'a> {
    /// What the widget is.
    pub role: Role,
    /// Its name: the text a person would call it by. A widget without one
    /// is named after the text of its children.
    pub label: Option<&'a str>,
    /// What it shows while it is empty (a field) or nothing is chosen (a
    /// list): an example, not a name. A widget without a name is called by
    /// it.
    pub placeholder: Option<&'a str>,
    /// Its value as text: the chosen option of a list.
    pub value: Option<&'a str>,
    /// Whether it is ticked, for what can be.
    pub toggled: Option<Toggled>,
    /// Its value as a number with the least and the most it can be.
    pub range: Option<(f32, f32, f32)>,
    /// How far one step moves the number of [`range`](Self::range).
    pub step: Option<f32>,
    /// The caret and the selection in [`value`](Self::value), for what
    /// holds text.
    pub selection: Option<Selection>,
    /// Whether what it opens (its list, its menu) is open, for what opens
    /// something.
    pub expanded: Option<bool>,
    /// Whether it is the chosen one, for an option among others.
    pub selected: Option<bool>,
    /// Whether, and how, a change of its text is said without being asked
    /// for.
    pub live: Option<Live>,
    /// Whether it can be used.
    pub is_enabled: bool,
}

impl<'a> Accessible<'a> {
    /// Creates the description of an enabled widget with the given [`Role`].
    pub fn new(role: Role) -> Self {
        Self {
            role,
            label: None,
            placeholder: None,
            value: None,
            toggled: None,
            range: None,
            step: None,
            selection: None,
            expanded: None,
            selected: None,
            live: None,
            is_enabled: true,
        }
    }

    /// Sets the name of the widget.
    pub fn label(mut self, label: impl Into<Option<&'a str>>) -> Self {
        self.label = label.into();
        self
    }

    /// Sets what the widget shows while it is empty.
    pub fn placeholder(
        mut self,
        placeholder: impl Into<Option<&'a str>>,
    ) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the value of the widget, as text.
    pub fn value(mut self, value: impl Into<Option<&'a str>>) -> Self {
        self.value = value.into();
        self
    }

    /// Sets whether the widget is ticked.
    pub fn toggled(mut self, toggled: impl Into<Toggled>) -> Self {
        self.toggled = Some(toggled.into());
        self
    }

    /// Sets the value of the widget as a number, with the least and the
    /// most it can be.
    pub fn range(mut self, value: f32, min: f32, max: f32) -> Self {
        self.range = Some((value, min, max));
        self
    }

    /// Sets how far one step moves the number of the widget.
    pub fn step(mut self, step: f32) -> Self {
        self.step = Some(step);
        self
    }

    /// Sets the caret and the selection in the value of the widget.
    pub fn selection(
        mut self,
        selection: impl Into<Option<Selection>>,
    ) -> Self {
        self.selection = selection.into();
        self
    }

    /// Sets whether what the widget opens is open.
    pub fn expanded(mut self, is_expanded: bool) -> Self {
        self.expanded = Some(is_expanded);
        self
    }

    /// Sets whether what the widget opens is open, for a widget that may
    /// open nothing.
    pub fn expanded_maybe(mut self, is_expanded: Option<bool>) -> Self {
        self.expanded = is_expanded;
        self
    }

    /// Sets whether the widget is the chosen one among others.
    pub fn selected(mut self, is_selected: bool) -> Self {
        self.selected = Some(is_selected);
        self
    }

    /// Sets how a change of the text of the widget is said.
    pub fn live(mut self, live: impl Into<Option<Live>>) -> Self {
        self.live = live.into();
        self
    }

    /// Sets whether the widget can be used.
    pub fn enabled(mut self, is_enabled: bool) -> Self {
        self.is_enabled = is_enabled;
        self
    }
}
